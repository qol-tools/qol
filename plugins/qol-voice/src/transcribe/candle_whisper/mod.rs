mod model;

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Sender, SyncSender, TrySendError};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crate::audio::{AudioEncoding, AudioFormat, AudioFrame};

use super::{
    AudioSubmit, ProviderLocation, Transcriber, TranscriberCapabilities, TranscriberDescriptor,
    TranscriberRegistration, TranscriptionError, TranscriptionEvent, TranscriptionSession,
};
use model::{append_pcm, WhisperDecoder};

const COMMAND_QUEUE_CAPACITY: usize = 64;

pub(crate) const REGISTRATION: TranscriberRegistration = TranscriberRegistration {
    descriptor: TranscriberDescriptor {
        id: "candle-whisper",
        name: "Candle Whisper",
        capabilities: TranscriberCapabilities {
            partial_results: false,
            ordered_finalization: true,
            word_timestamps: false,
            language_detection: false,
            location: ProviderLocation::Local,
        },
    },
    auto_select: true,
    create: create_from_options,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandleWhisperConfig {
    pub model_id: String,
    pub revision: String,
    pub language: Option<String>,
}

impl Default for CandleWhisperConfig {
    fn default() -> Self {
        Self {
            model_id: model::DEFAULT_MODEL_ID.to_owned(),
            revision: model::DEFAULT_REVISION.to_owned(),
            language: None,
        }
    }
}

pub struct CandleWhisperTranscriber {
    config: CandleWhisperConfig,
}

impl CandleWhisperTranscriber {
    pub fn new(config: CandleWhisperConfig) -> Self {
        Self { config }
    }
}

fn create_from_options(
    options: &BTreeMap<String, String>,
) -> Result<Box<dyn Transcriber>, TranscriptionError> {
    reject_unknown_options(options)?;
    let mut config = CandleWhisperConfig::default();
    if let Some(model_id) = options.get("model_id") {
        config.model_id = model_id.clone();
        config.revision = "main".to_owned();
    }
    if let Some(revision) = options.get("revision") {
        config.revision = revision.clone();
    }
    config.language = options.get("language").cloned();
    Ok(Box::new(CandleWhisperTranscriber::new(config)))
}

fn reject_unknown_options(options: &BTreeMap<String, String>) -> Result<(), TranscriptionError> {
    let accepted = ["model_id", "revision", "language"];
    let Some(option) = options
        .keys()
        .find(|option| !accepted.contains(&option.as_str()))
    else {
        return Ok(());
    };
    Err(TranscriptionError::InvalidConfiguration(format!(
        "Candle Whisper provider does not recognize option {option}"
    )))
}

enum Command {
    Audio(AudioFrame),
    Finalize,
    Shutdown,
}

struct CandleWhisperSession {
    commands: SyncSender<Command>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl TranscriptionSession for CandleWhisperSession {
    fn submit_audio(&self, frame: AudioFrame) -> Result<AudioSubmit, TranscriptionError> {
        if !frame.pcm.len().is_multiple_of(2) {
            return Err(TranscriptionError::InferenceFailed(
                "PCM_S16LE audio contains an incomplete sample".to_owned(),
            ));
        }
        match self.commands.try_send(Command::Audio(frame)) {
            Ok(()) => Ok(AudioSubmit::Accepted),
            Err(TrySendError::Full(_)) => Ok(AudioSubmit::Dropped),
            Err(TrySendError::Disconnected(_)) => Err(TranscriptionError::StreamClosed(
                "local transcription input is closed".to_owned(),
            )),
        }
    }

    fn finalize_user_turn(&self) -> Result<(), TranscriptionError> {
        self.commands.send(Command::Finalize).map_err(|_| {
            TranscriptionError::StreamClosed("local transcription input is closed".to_owned())
        })
    }
}

impl Drop for CandleWhisperSession {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        let Ok(worker) = self.worker.get_mut() else {
            return;
        };
        let Some(worker) = worker.take() else {
            return;
        };
        let _ = worker.join();
    }
}

impl Transcriber for CandleWhisperTranscriber {
    fn start(
        &self,
        format: AudioFormat,
        session_started_at: Instant,
        events: Sender<Result<TranscriptionEvent, TranscriptionError>>,
    ) -> Result<Box<dyn TranscriptionSession>, TranscriptionError> {
        validate_format(format)?;
        let (commands, receiver) = mpsc::sync_channel(COMMAND_QUEUE_CAPACITY);
        let config = self.config.clone();
        let worker = thread::Builder::new()
            .name("qol-voice-candle-whisper".to_owned())
            .spawn(move || run_worker(config, receiver, session_started_at, events))
            .map_err(|error| TranscriptionError::ProviderUnavailable(error.to_string()))?;
        Ok(Box::new(CandleWhisperSession {
            commands,
            worker: Mutex::new(Some(worker)),
        }))
    }
}

fn validate_format(format: AudioFormat) -> Result<(), TranscriptionError> {
    if format.sample_rate != model::SAMPLE_RATE {
        return Err(TranscriptionError::InvalidConfiguration(format!(
            "Candle Whisper requires {} Hz audio",
            model::SAMPLE_RATE
        )));
    }
    if format.channels != 1 {
        return Err(TranscriptionError::InvalidConfiguration(
            "Candle Whisper requires mono audio".to_owned(),
        ));
    }
    if format.encoding != AudioEncoding::PcmS16Le {
        return Err(TranscriptionError::InvalidConfiguration(
            "Candle Whisper requires PCM_S16LE audio".to_owned(),
        ));
    }
    Ok(())
}

fn run_worker(
    config: CandleWhisperConfig,
    commands: mpsc::Receiver<Command>,
    session_started_at: Instant,
    events: Sender<Result<TranscriptionEvent, TranscriptionError>>,
) {
    let load_started_at = Instant::now();
    qol_runtime::probe!(
        "VOICE_SESSION",
        "event=transcriber_loading provider=candle-whisper model={}",
        config.model_id
    );
    let mut decoder = match WhisperDecoder::load(&config) {
        Ok(decoder) => {
            qol_runtime::probe!(
                "VOICE_SESSION",
                "event=transcriber_ready provider=candle-whisper elapsed_ms={}",
                elapsed_ms(load_started_at)
            );
            decoder
        }
        Err(error) => {
            qol_runtime::probe!(
                "VOICE_SESSION",
                "event=transcriber_failed provider=candle-whisper elapsed_ms={} error={}",
                elapsed_ms(load_started_at),
                error
            );
            let _ = events.send(Err(error));
            return;
        }
    };
    let mut audio = Vec::new();
    let mut observed_at_ms = 0;
    while let Ok(command) = commands.recv() {
        match command {
            Command::Audio(frame) => {
                observed_at_ms = frame.observed_at_ms;
                append_pcm(&mut audio, &frame.pcm);
            }
            Command::Finalize if audio.is_empty() => {}
            Command::Finalize => {
                let inference_started_at = Instant::now();
                let audio_samples = audio.len();
                qol_runtime::probe!(
                    "VOICE_SESSION",
                    "event=transcription_started provider=candle-whisper audio_samples={audio_samples}"
                );
                let result = decoder.transcribe(&audio).map(|text| TranscriptionEvent {
                    observed_at_ms: observed_at_ms.max(elapsed_ms(session_started_at)),
                    text,
                    confidence_permille: None,
                    final_result: true,
                });
                audio.clear();
                match result {
                    Ok(event) => {
                        qol_runtime::probe!(
                            "VOICE_SESSION",
                            "event=transcription_completed provider=candle-whisper elapsed_ms={} characters={}",
                            elapsed_ms(inference_started_at),
                            event.text.chars().count()
                        );
                        if events.send(Ok(event)).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        qol_runtime::probe!(
                            "VOICE_SESSION",
                            "event=transcription_failed provider=candle-whisper elapsed_ms={} error={}",
                            elapsed_ms(inference_started_at),
                            error
                        );
                        let _ = events.send(Err(error));
                    }
                }
            }
            Command::Shutdown => return,
        }
    }
}

fn elapsed_ms(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{create_from_options, CandleWhisperConfig};

    #[test]
    fn provider_options_are_isolated_from_core_selection() {
        let options = BTreeMap::from([
            ("model_id".to_owned(), "owner/model".to_owned()),
            ("revision".to_owned(), "v1".to_owned()),
            ("language".to_owned(), "da".to_owned()),
        ]);
        assert!(create_from_options(&options).is_ok());
        assert_eq!(
            CandleWhisperConfig::default().model_id,
            "openai/whisper-tiny.en"
        );
    }

    #[test]
    fn rejects_unknown_provider_options() {
        let options = BTreeMap::from([("endpoint".to_owned(), "ignored".to_owned())]);
        assert!(create_from_options(&options).is_err());
    }
}
