mod protocol;

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Sender};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc::{
    self as tokio_mpsc, error::TrySendError, UnboundedReceiver, UnboundedSender,
};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;

use crate::audio::{AudioFormat, AudioFrame};

use super::{
    AudioSubmit, ProviderLocation, Transcriber, TranscriberCapabilities, TranscriberDescriptor,
    TranscriberRegistration, TranscriptionError, TranscriptionEvent, TranscriptionSession,
};
use protocol::{config_message, end_of_turn_message, into_event, parse_message, ServerMessage};

const AUDIO_QUEUE_CAPACITY: usize = 50;
const IO_TIMEOUT: Duration = Duration::from_secs(5);

pub(crate) const REGISTRATION: TranscriberRegistration = TranscriberRegistration {
    descriptor: TranscriberDescriptor {
        id: "websocket",
        name: "WebSocket compatibility provider",
        capabilities: TranscriberCapabilities {
            partial_results: true,
            ordered_finalization: true,
            word_timestamps: false,
            language_detection: false,
            location: ProviderLocation::Remote,
        },
    },
    auto_select: false,
    create: create_from_options,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WebSocketTranscriberConfig {
    pub endpoint: String,
    pub engine: String,
    pub connect_timeout: Duration,
}

impl Default for WebSocketTranscriberConfig {
    fn default() -> Self {
        Self {
            endpoint: "ws://127.0.0.1:5001".to_owned(),
            engine: "whisper".to_owned(),
            connect_timeout: Duration::from_secs(5),
        }
    }
}

pub struct WebSocketTranscriber {
    config: WebSocketTranscriberConfig,
}

impl WebSocketTranscriber {
    pub fn new(config: WebSocketTranscriberConfig) -> Self {
        Self { config }
    }
}

fn create_from_options(
    options: &BTreeMap<String, String>,
) -> Result<Box<dyn Transcriber>, TranscriptionError> {
    reject_unknown_options(options, &["endpoint", "engine", "connect_timeout_ms"])?;
    let endpoint = options.get("endpoint").cloned().ok_or_else(|| {
        TranscriptionError::InvalidConfiguration(
            "WebSocket provider requires option endpoint=ws://...".to_owned(),
        )
    })?;
    let engine = options
        .get("engine")
        .cloned()
        .unwrap_or_else(|| "whisper".to_owned());
    let connect_timeout = options
        .get("connect_timeout_ms")
        .map(|value| parse_timeout(value))
        .transpose()?
        .unwrap_or(Duration::from_secs(5));
    Ok(Box::new(WebSocketTranscriber::new(
        WebSocketTranscriberConfig {
            endpoint,
            engine,
            connect_timeout,
        },
    )))
}

fn reject_unknown_options(
    options: &BTreeMap<String, String>,
    accepted: &[&str],
) -> Result<(), TranscriptionError> {
    let Some(option) = options
        .keys()
        .find(|option| !accepted.contains(&option.as_str()))
    else {
        return Ok(());
    };
    Err(TranscriptionError::InvalidConfiguration(format!(
        "WebSocket provider does not recognize option {option}"
    )))
}

fn parse_timeout(value: &str) -> Result<Duration, TranscriptionError> {
    let milliseconds = value.parse::<u64>().map_err(|_| {
        TranscriptionError::InvalidConfiguration(
            "connect_timeout_ms must be a positive integer".to_owned(),
        )
    })?;
    if milliseconds == 0 {
        return Err(TranscriptionError::InvalidConfiguration(
            "connect_timeout_ms must be a positive integer".to_owned(),
        ));
    }
    Ok(Duration::from_millis(milliseconds))
}

enum Control {
    FinalizeUserTurn { after_audio_sequence: u64 },
    Shutdown,
}

struct SequencedAudio {
    sequence: u64,
    frame: AudioFrame,
}

struct WebSocketTranscriptionSession {
    audio: tokio_mpsc::Sender<SequencedAudio>,
    audio_sequence: Mutex<u64>,
    control: UnboundedSender<Control>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl TranscriptionSession for WebSocketTranscriptionSession {
    fn submit_audio(&self, frame: AudioFrame) -> Result<AudioSubmit, TranscriptionError> {
        let mut audio_sequence = self.audio_sequence.lock().map_err(|_| {
            TranscriptionError::StreamClosed("audio sequence lock is unavailable".to_owned())
        })?;
        let sequence = audio_sequence.saturating_add(1);
        match self.audio.try_send(SequencedAudio { sequence, frame }) {
            Ok(()) => {
                *audio_sequence = sequence;
                Ok(AudioSubmit::Accepted)
            }
            Err(TrySendError::Full(_)) => Ok(AudioSubmit::Dropped),
            Err(TrySendError::Closed(_)) => Err(TranscriptionError::StreamClosed(
                "audio input is no longer accepted".to_owned(),
            )),
        }
    }

    fn finalize_user_turn(&self) -> Result<(), TranscriptionError> {
        let after_audio_sequence = *self.audio_sequence.lock().map_err(|_| {
            TranscriptionError::StreamClosed("audio sequence lock is unavailable".to_owned())
        })?;
        self.control
            .send(Control::FinalizeUserTurn {
                after_audio_sequence,
            })
            .map_err(|_| TranscriptionError::StreamClosed("control input is closed".to_owned()))
    }
}

impl Drop for WebSocketTranscriptionSession {
    fn drop(&mut self) {
        let _ = self.control.send(Control::Shutdown);
        let Ok(worker) = self.worker.get_mut() else {
            return;
        };
        let Some(worker) = worker.take() else {
            return;
        };
        let _ = worker.join();
    }
}

impl Transcriber for WebSocketTranscriber {
    fn start(
        &self,
        format: AudioFormat,
        session_started_at: Instant,
        events: Sender<Result<TranscriptionEvent, TranscriptionError>>,
    ) -> Result<Box<dyn TranscriptionSession>, TranscriptionError> {
        validate_config(&self.config, format)?;
        let (audio_sender, audio_receiver) = tokio_mpsc::channel(AUDIO_QUEUE_CAPACITY);
        let (control_sender, control_receiver) = tokio_mpsc::unbounded_channel();
        let (startup_sender, startup_receiver) = mpsc::sync_channel(1);
        let config = self.config.clone();
        let worker = thread::Builder::new()
            .name("qol-voice-transcription".to_owned())
            .spawn(move || {
                run_worker(
                    config,
                    format,
                    session_started_at,
                    audio_receiver,
                    control_receiver,
                    startup_sender,
                    events,
                );
            })
            .map_err(|error| TranscriptionError::ConnectionFailed(error.to_string()))?;
        let startup = startup_receiver.recv().map_err(|_| {
            TranscriptionError::ConnectionFailed(
                "transcription worker stopped during startup".to_owned(),
            )
        })?;
        if let Err(error) = startup {
            let _ = worker.join();
            return Err(error);
        }
        Ok(Box::new(WebSocketTranscriptionSession {
            audio: audio_sender,
            audio_sequence: Mutex::new(0),
            control: control_sender,
            worker: Mutex::new(Some(worker)),
        }))
    }
}

fn validate_config(
    config: &WebSocketTranscriberConfig,
    format: AudioFormat,
) -> Result<(), TranscriptionError> {
    if config.endpoint.trim().is_empty() {
        return Err(TranscriptionError::InvalidConfiguration(
            "WebSocket endpoint cannot be empty".to_owned(),
        ));
    }
    if config.engine.trim().is_empty() {
        return Err(TranscriptionError::InvalidConfiguration(
            "engine cannot be empty".to_owned(),
        ));
    }
    if format.sample_rate == 0 || format.channels == 0 {
        return Err(TranscriptionError::InvalidConfiguration(
            "audio format must have a sample rate and channel count".to_owned(),
        ));
    }
    Ok(())
}

fn run_worker(
    config: WebSocketTranscriberConfig,
    format: AudioFormat,
    session_started_at: Instant,
    audio: tokio_mpsc::Receiver<SequencedAudio>,
    control: UnboundedReceiver<Control>,
    startup: mpsc::SyncSender<Result<(), TranscriptionError>>,
    events: Sender<Result<TranscriptionEvent, TranscriptionError>>,
) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    let Ok(runtime) = runtime else {
        let _ = startup.send(Err(TranscriptionError::ConnectionFailed(
            "could not start async runtime".to_owned(),
        )));
        return;
    };
    let result = runtime.block_on(run_websocket(
        config,
        format,
        session_started_at,
        audio,
        control,
        startup.clone(),
        events.clone(),
    ));
    if let Err(error) = result {
        if startup.send(Err(error.clone())).is_err() {
            let _ = events.send(Err(error));
        }
    }
}

async fn run_websocket(
    config: WebSocketTranscriberConfig,
    format: AudioFormat,
    session_started_at: Instant,
    mut audio: tokio_mpsc::Receiver<SequencedAudio>,
    mut control: UnboundedReceiver<Control>,
    startup: mpsc::SyncSender<Result<(), TranscriptionError>>,
    events: Sender<Result<TranscriptionEvent, TranscriptionError>>,
) -> Result<(), TranscriptionError> {
    let connection = timeout(
        config.connect_timeout,
        tokio_tungstenite::connect_async(&config.endpoint),
    )
    .await
    .map_err(|_| {
        TranscriptionError::ConnectionFailed(format!("timed out connecting to {}", config.endpoint))
    })?
    .map_err(|error| TranscriptionError::ConnectionFailed(error.to_string()))?;
    let (mut socket, _) = connection;
    send_message(
        &mut socket,
        Message::text(config_message(format, &config.engine)?),
    )
    .await?;
    await_configuration_ack(&mut socket, session_started_at, &events).await?;
    if startup.send(Ok(())).is_err() {
        let _ = socket.close(None).await;
        return Ok(());
    }
    let mut audio_open = true;
    let mut last_audio_sequence = 0_u64;
    let mut pending_finalize = None;
    loop {
        if pending_finalize.is_some_and(|target| last_audio_sequence >= target) {
            send_message(&mut socket, Message::text(end_of_turn_message()?)).await?;
            pending_finalize = None;
            continue;
        }
        tokio::select! {
            biased;
            control_message = control.recv() => {
                match control_message {
                    Some(Control::FinalizeUserTurn { after_audio_sequence }) => {
                        pending_finalize = Some(after_audio_sequence);
                    }
                    Some(Control::Shutdown) | None => {
                        let _ = timeout(IO_TIMEOUT, socket.close(None)).await;
                        return Ok(());
                    }
                }
            }
            incoming = socket.next() => {
                handle_incoming(incoming, session_started_at, &events)?;
            }
            sequenced_audio = audio.recv(), if audio_open => {
                let Some(sequenced_audio) = sequenced_audio else {
                    audio_open = false;
                    continue;
                };
                send_message(&mut socket, Message::binary(sequenced_audio.frame.pcm)).await?;
                last_audio_sequence = sequenced_audio.sequence;
            }
        }
    }
}

async fn await_configuration_ack<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    session_started_at: Instant,
    events: &Sender<Result<TranscriptionEvent, TranscriptionError>>,
) -> Result<(), TranscriptionError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    timeout(IO_TIMEOUT, async {
        loop {
            let incoming = socket.next().await;
            let Some(message) = incoming else {
                return Err(TranscriptionError::ConnectionFailed(
                    "server closed before accepting configuration".to_owned(),
                ));
            };
            let message =
                message.map_err(|error| TranscriptionError::ConnectionFailed(error.to_string()))?;
            let Message::Text(raw) = message else {
                continue;
            };
            let parsed = parse_message(raw.as_str())?;
            if parsed == ServerMessage::ConfigurationAccepted {
                return Ok(());
            }
            if let Some(event) = into_event(parsed, elapsed_ms(session_started_at)) {
                let _ = events.send(Ok(event));
            }
        }
    })
    .await
    .map_err(|_| {
        TranscriptionError::ConnectionFailed(
            "server did not acknowledge configuration within 5 seconds".to_owned(),
        )
    })?
}

fn handle_incoming(
    incoming: Option<Result<Message, tokio_tungstenite::tungstenite::Error>>,
    session_started_at: Instant,
    events: &Sender<Result<TranscriptionEvent, TranscriptionError>>,
) -> Result<(), TranscriptionError> {
    let Some(message) = incoming else {
        return Err(TranscriptionError::StreamClosed(
            "server ended the WebSocket stream".to_owned(),
        ));
    };
    let message = message.map_err(|error| TranscriptionError::StreamClosed(error.to_string()))?;
    match message {
        Message::Text(raw) => {
            let parsed = parse_message(raw.as_str())?;
            let Some(event) = into_event(parsed, elapsed_ms(session_started_at)) else {
                return Ok(());
            };
            events
                .send(Ok(event))
                .map_err(|_| TranscriptionError::StreamClosed("event receiver closed".to_owned()))
        }
        Message::Close(_) => Err(TranscriptionError::StreamClosed(
            "server closed the WebSocket connection".to_owned(),
        )),
        Message::Binary(_) => Err(TranscriptionError::ProtocolFailed(
            "server returned an unexpected binary message".to_owned(),
        )),
        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => Ok(()),
    }
}

async fn send_message<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    message: Message,
) -> Result<(), TranscriptionError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    timeout(IO_TIMEOUT, socket.send(message))
        .await
        .map_err(|_| TranscriptionError::StreamClosed("WebSocket send timed out".to_owned()))?
        .map_err(|error| TranscriptionError::StreamClosed(error.to_string()))
}

fn elapsed_ms(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    use crate::audio::{AudioEncoding, AudioFormat, AudioFrame};
    use crate::transcribe::{AudioSubmit, Transcriber};

    use super::{WebSocketTranscriber, WebSocketTranscriberConfig};

    #[test]
    fn streams_audio_and_orders_finalize_at_the_accepted_boundary() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("ws://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || run_fake_server(listener));
        let transcriber = WebSocketTranscriber::new(WebSocketTranscriberConfig {
            endpoint,
            engine: "whisper".to_owned(),
            connect_timeout: Duration::from_secs(2),
        });
        let (events, receiver) = mpsc::channel();
        let session = transcriber
            .start(
                AudioFormat {
                    sample_rate: 16_000,
                    channels: 1,
                    encoding: AudioEncoding::PcmS16Le,
                },
                Instant::now(),
                events,
            )
            .unwrap();

        assert_eq!(
            session
                .submit_audio(AudioFrame {
                    observed_at_ms: 100,
                    pcm: vec![1, 2, 3, 4],
                })
                .unwrap(),
            AudioSubmit::Accepted
        );
        session.finalize_user_turn().unwrap();
        let partial = receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        let final_result = receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        assert_eq!(partial.text, "hello");
        assert!(!partial.final_result);
        assert_eq!(final_result.text, "hello there");
        assert!(final_result.final_result);
        drop(session);

        let (config, audio, end_of_turn) = server.join().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&config).unwrap(),
            serde_json::json!({
                "type": "config",
                "config": {
                    "sample_rate": 16000,
                    "channels": 1,
                    "encoding": "PCM_S16LE",
                    "engine": "whisper"
                }
            })
        );
        assert_eq!(audio, vec![1, 2, 3, 4]);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&end_of_turn).unwrap(),
            serde_json::json!({"eof": 1})
        );
    }

    fn run_fake_server(listener: TcpListener) -> (String, Vec<u8>, String) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let config = next_text(&mut socket).await;
                socket
                    .send(Message::text(
                        r#"{"status":"ok","message":"configuration accepted"}"#,
                    ))
                    .await
                    .unwrap();
                let audio = next_binary(&mut socket).await;
                socket
                    .send(Message::text(
                        r#"{"status":"ok","result_type":"partial","result":{"hypotheses":[{"transcript":"hello"}],"final":false}}"#,
                    ))
                    .await
                    .unwrap();
                let end_of_turn = next_text(&mut socket).await;
                socket
                    .send(Message::text(
                        r#"{"status":"ok","result_type":"final","result":{"hypotheses":[{"transcript":"hello there"}],"final":true}}"#,
                    ))
                    .await
                    .unwrap();
                let _ = socket.next().await;
                (config, audio, end_of_turn)
            })
    }

    async fn next_text<S>(socket: &mut tokio_tungstenite::WebSocketStream<S>) -> String
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match message {
            Message::Text(text) => text.to_string(),
            other => panic!("expected text message, received {other:?}"),
        }
    }

    async fn next_binary<S>(socket: &mut tokio_tungstenite::WebSocketStream<S>) -> Vec<u8>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        match message {
            Message::Binary(bytes) => bytes.to_vec(),
            other => panic!("expected binary message, received {other:?}"),
        }
    }
}
