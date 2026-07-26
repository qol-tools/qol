mod pipeline;
mod platform;
mod recognition;
mod utterance;

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::{unbounded, Receiver, Sender};

use pipeline::{audio_frame_channel, AudioPipeline};
use platform::PlatformAudioInput;
use recognition::{start_transcription, RecognitionController, TranscriptionBridge};

use crate::audio::{AudioFormat, AudioFrame};
use crate::transcribe::Transcriber;
use crate::turn::TurnId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListenConfig {
    pub threshold_permille: u16,
    pub onset_ms: u64,
    pub silence_ms: u64,
    pub pre_roll_ms: u64,
    pub max_utterance_ms: u64,
}

impl Default for ListenConfig {
    fn default() -> Self {
        Self {
            threshold_permille: 10,
            onset_ms: 100,
            silence_ms: 700,
            pre_roll_ms: 300,
            max_utterance_ms: 30_000,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AudioInputRequest {
    pub device_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AudioInputInfo {
    pub device_name: String,
    pub format: AudioFormat,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioInputDevice {
    pub id: String,
    pub label: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioInputProbe {
    pub device_id: String,
    pub captured_ms: u64,
    pub peak_permille: u16,
    pub rms_permille: u16,
    pub nonzero_permille: u16,
    pub clipped_samples: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioDropStage {
    Capture,
    Transcription,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UtteranceEndReason {
    Silence,
    MaximumDuration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ListenEvent {
    VoiceActivityStarted {
        turn_id: TurnId,
        observed_at_ms: u64,
        level_permille: u16,
    },
    VoiceActivityEnded {
        turn_id: TurnId,
        observed_at_ms: u64,
        level_permille: u16,
        reason: UtteranceEndReason,
    },
    TranscriptHypothesis {
        turn_id: TurnId,
        observed_at_ms: u64,
        text: String,
        confidence_permille: Option<u16>,
        final_result: bool,
    },
    AudioFramesDropped {
        observed_at_ms: u64,
        stage: AudioDropStage,
        count: u64,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ListenError {
    NoInputDevice,
    InputUnavailable(String),
    CaptureFailed(String),
    TranscriptionFailed(String),
    EventChannelClosed,
}

impl Display for ListenError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoInputDevice => write!(formatter, "no default microphone is available"),
            Self::InputUnavailable(message) => {
                write!(formatter, "microphone unavailable: {message}")
            }
            Self::CaptureFailed(message) => {
                write!(formatter, "microphone capture failed: {message}")
            }
            Self::TranscriptionFailed(message) => {
                write!(formatter, "transcription failed: {message}")
            }
            Self::EventChannelClosed => write!(formatter, "microphone event channel closed"),
        }
    }
}

impl Error for ListenError {}

pub fn audio_input_devices() -> Result<Vec<AudioInputDevice>, ListenError> {
    platform::audio_input_devices()
}

pub fn verify_audio_input() -> Result<(), ListenError> {
    platform::verify_audio_input()
}

pub fn probe_audio_input(
    input: AudioInputRequest,
    duration_ms: u64,
) -> Result<AudioInputProbe, ListenError> {
    platform::probe_audio_input(input, duration_ms)
}

pub struct ListeningSession {
    info: AudioInputInfo,
    receiver: Receiver<ListenMessage>,
    sender: Sender<ListenMessage>,
    _input: Box<dyn ActiveAudioInput>,
    _pipeline: AudioPipeline,
    recognition: Arc<RecognitionController>,
    _transcription_bridge: Option<TranscriptionBridge>,
}

impl ListeningSession {
    pub fn start(input: AudioInputRequest, config: ListenConfig) -> Result<Self, ListenError> {
        Self::start_with(&PlatformAudioInput::new(input), config, None)
    }

    pub fn start_transcribed(
        input: AudioInputRequest,
        config: ListenConfig,
        transcriber: &dyn Transcriber,
    ) -> Result<Self, ListenError> {
        Self::start_with(&PlatformAudioInput::new(input), config, Some(transcriber))
    }

    fn start_with(
        input: &dyn AudioInput,
        config: ListenConfig,
        transcriber: Option<&dyn Transcriber>,
    ) -> Result<Self, ListenError> {
        let session_started_at = Instant::now();
        let (sender, receiver) = unbounded();
        let info = input.info()?;
        let (recognition, transcription_bridge) =
            start_transcription(transcriber, info.format, session_started_at, sender.clone())?;
        let (frame_sender, frame_receiver, capture_drops) = audio_frame_channel();
        let pipeline = AudioPipeline::start(
            config,
            info.format,
            frame_receiver,
            capture_drops.clone(),
            recognition.clone(),
            sender.clone(),
        )?;
        let active_input = input.start(
            session_started_at,
            frame_sender,
            capture_drops,
            sender.clone(),
        )?;
        Ok(Self {
            info,
            receiver,
            sender,
            _input: active_input,
            _pipeline: pipeline,
            recognition,
            _transcription_bridge: transcription_bridge,
        })
    }

    pub fn info(&self) -> &AudioInputInfo {
        &self.info
    }

    pub fn stop_handle(&self) -> ListenStopHandle {
        ListenStopHandle {
            sender: self.sender.clone(),
        }
    }

    pub fn receive(&self) -> Result<Option<ListenEvent>, ListenError> {
        self.receiver
            .recv()
            .map_err(|_| ListenError::EventChannelClosed)?
    }

    pub(crate) fn event_receiver(&self) -> &Receiver<ListenMessage> {
        &self.receiver
    }

    pub fn finalize_user_turn(&self, turn_id: TurnId) -> Result<(), ListenError> {
        self.recognition
            .finalize_turn(turn_id)
            .map_err(|error| ListenError::TranscriptionFailed(error.to_string()))
    }
}

#[derive(Clone)]
pub struct ListenStopHandle {
    sender: Sender<ListenMessage>,
}

impl ListenStopHandle {
    pub fn stop(&self) {
        let _ = self.sender.send(Ok(None));
    }
}

pub(crate) type ListenMessage = Result<Option<ListenEvent>, ListenError>;

trait ActiveAudioInput: Send {}

trait AudioInput {
    fn info(&self) -> Result<AudioInputInfo, ListenError>;

    fn start(
        &self,
        session_started_at: Instant,
        frames: SyncSender<AudioFrame>,
        dropped: Arc<AtomicU64>,
        events: Sender<ListenMessage>,
    ) -> Result<Box<dyn ActiveAudioInput>, ListenError>;
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use crossbeam_channel::Sender;

    use crate::audio::{AudioEncoding, AudioFormat, AudioFrame};
    use crate::transcribe::{
        AudioSubmit, Transcriber, TranscriptionError, TranscriptionEvent, TranscriptionSession,
    };
    use crate::turn::TurnId;

    use super::{
        ActiveAudioInput, AudioInput, AudioInputInfo, ListenConfig, ListenError, ListenEvent,
        ListenMessage, ListeningSession, UtteranceEndReason,
    };

    struct FakeAudioInput {
        frames: Vec<AudioFrame>,
        error: Option<ListenError>,
    }

    struct FakeActiveAudioInput;

    impl ActiveAudioInput for FakeActiveAudioInput {}

    impl AudioInput for FakeAudioInput {
        fn info(&self) -> Result<AudioInputInfo, ListenError> {
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            Ok(AudioInputInfo {
                device_name: "fake microphone".to_owned(),
                format: AudioFormat {
                    sample_rate: 1_000,
                    channels: 1,
                    encoding: AudioEncoding::PcmS16Le,
                },
            })
        }

        fn start(
            &self,
            _session_started_at: std::time::Instant,
            frames: std::sync::mpsc::SyncSender<AudioFrame>,
            _dropped: Arc<std::sync::atomic::AtomicU64>,
            _events: Sender<ListenMessage>,
        ) -> Result<Box<dyn ActiveAudioInput>, ListenError> {
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            for frame in self.frames.clone() {
                frames.try_send(frame).unwrap();
            }
            Ok(Box::new(FakeActiveAudioInput))
        }
    }

    struct FakeTranscriber {
        finalized: Arc<AtomicBool>,
        submitted: Arc<Mutex<Vec<AudioFrame>>>,
    }

    struct FakeTranscriptionSession {
        finalized: Arc<AtomicBool>,
        submitted: Arc<Mutex<Vec<AudioFrame>>>,
        events: std::sync::mpsc::Sender<Result<TranscriptionEvent, TranscriptionError>>,
    }

    impl TranscriptionSession for FakeTranscriptionSession {
        fn submit_audio(&self, frame: AudioFrame) -> Result<AudioSubmit, TranscriptionError> {
            let observed_at_ms = frame.observed_at_ms;
            self.submitted.lock().unwrap().push(frame);
            self.events
                .send(Ok(TranscriptionEvent {
                    observed_at_ms,
                    text: "hello".to_owned(),
                    confidence_permille: None,
                    final_result: false,
                }))
                .unwrap();
            Ok(AudioSubmit::Accepted)
        }

        fn finalize_user_turn(&self) -> Result<(), TranscriptionError> {
            self.finalized.store(true, Ordering::Release);
            Ok(())
        }
    }

    impl Transcriber for FakeTranscriber {
        fn start(
            &self,
            _format: AudioFormat,
            _session_started_at: std::time::Instant,
            events: std::sync::mpsc::Sender<Result<TranscriptionEvent, TranscriptionError>>,
        ) -> Result<Box<dyn TranscriptionSession>, TranscriptionError> {
            Ok(Box::new(FakeTranscriptionSession {
                finalized: self.finalized.clone(),
                submitted: self.submitted.clone(),
                events,
            }))
        }
    }

    #[test]
    fn session_derives_vad_from_adapter_frames() {
        let session = ListeningSession::start_with(
            &FakeAudioInput {
                frames: vec![frame(100, 1_000, 20), frame(200, 0, 100)],
                error: None,
            },
            ListenConfig {
                threshold_permille: 10,
                onset_ms: 20,
                silence_ms: 100,
                pre_roll_ms: 20,
                max_utterance_ms: 1_000,
            },
            None,
        )
        .unwrap();

        assert_eq!(
            session.receive(),
            Ok(Some(ListenEvent::VoiceActivityStarted {
                turn_id: TurnId(1),
                observed_at_ms: 100,
                level_permille: 31,
            }))
        );
        assert_eq!(
            session.receive(),
            Ok(Some(ListenEvent::VoiceActivityEnded {
                turn_id: TurnId(1),
                observed_at_ms: 200,
                level_permille: 0,
                reason: UtteranceEndReason::Silence,
            }))
        );
    }

    #[test]
    fn session_routes_audio_events_and_finalize_through_transcriber() {
        let finalized = Arc::new(AtomicBool::new(false));
        let submitted = Arc::new(Mutex::new(Vec::new()));
        let transcriber = FakeTranscriber {
            finalized: finalized.clone(),
            submitted: submitted.clone(),
        };
        let session = ListeningSession::start_with(
            &FakeAudioInput {
                frames: vec![frame(100, 1_000, 20)],
                error: None,
            },
            ListenConfig {
                onset_ms: 20,
                ..ListenConfig::default()
            },
            Some(&transcriber),
        )
        .unwrap();

        let events = [
            session.receive().unwrap().unwrap(),
            session.receive().unwrap().unwrap(),
        ];
        assert!(events.iter().any(
            |event| matches!(event, ListenEvent::TranscriptHypothesis { text, .. } if text == "hello")
        ));
        assert!(events
            .iter()
            .any(|event| matches!(event, ListenEvent::VoiceActivityStarted { .. })));
        session.finalize_user_turn(TurnId(1)).unwrap();
        assert!(finalized.load(Ordering::Acquire));
        assert_eq!(submitted.lock().unwrap().len(), 1);
    }

    #[test]
    fn session_submits_preroll_before_live_recognition_audio() {
        let submitted = Arc::new(Mutex::new(Vec::new()));
        let transcriber = FakeTranscriber {
            finalized: Arc::new(AtomicBool::new(false)),
            submitted: submitted.clone(),
        };
        let session = ListeningSession::start_with(
            &FakeAudioInput {
                frames: vec![
                    frame(50, 0, 50),
                    frame(100, 1_000, 50),
                    frame(150, 1_000, 50),
                ],
                error: None,
            },
            ListenConfig {
                onset_ms: 100,
                pre_roll_ms: 150,
                ..ListenConfig::default()
            },
            Some(&transcriber),
        )
        .unwrap();

        for _ in 0..4 {
            session.receive().unwrap().unwrap();
        }

        let timestamps = submitted
            .lock()
            .unwrap()
            .iter()
            .map(|frame| frame.observed_at_ms)
            .collect::<Vec<_>>();
        assert_eq!(timestamps, vec![50, 100, 150]);
    }

    #[test]
    fn session_surfaces_input_start_failure() {
        let result = ListeningSession::start_with(
            &FakeAudioInput {
                frames: Vec::new(),
                error: Some(ListenError::CaptureFailed("disconnected".to_owned())),
            },
            ListenConfig::default(),
            None,
        );

        assert!(matches!(
            result,
            Err(ListenError::CaptureFailed(message)) if message == "disconnected"
        ));
    }

    fn frame(observed_at_ms: u64, sample: i16, samples: usize) -> AudioFrame {
        let pcm = std::iter::repeat_n(sample.to_le_bytes(), samples)
            .flatten()
            .collect();
        AudioFrame {
            observed_at_ms,
            pcm,
        }
    }
}
