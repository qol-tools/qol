mod layout;
mod recognizer;

use std::collections::BTreeMap;
use std::sync::mpsc::{self, Sender, SyncSender, TrySendError};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crate::audio::{AudioEncoding, AudioFormat, AudioFrame};
use crate::transcribe::{
    AudioSubmit, ProviderLocation, Transcriber, TranscriberCapabilities, TranscriberDescriptor,
    TranscriberRegistration, TranscriptionError, TranscriptionEvent, TranscriptionSession,
};

use layout::ModelKind;
use recognizer::{append_pcm, ModelRecognizer, SAMPLE_RATE};

const COMMAND_QUEUE_CAPACITY: usize = 64;

pub(crate) const REGISTRATION: TranscriberRegistration = TranscriberRegistration {
    descriptor: TranscriberDescriptor {
        id: "sherpa-onnx",
        name: "Sherpa ONNX",
        capabilities: TranscriberCapabilities {
            partial_results: false,
            ordered_finalization: true,
            word_timestamps: false,
            language_detection: false,
            location: ProviderLocation::Local,
        },
    },
    auto_select: false,
    create: create_from_options,
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct SherpaOnnxConfig {
    model_dir: String,
    model_kind: Option<ModelKind>,
    language: Option<String>,
    threads: i32,
}

pub struct SherpaOnnxTranscriber {
    config: SherpaOnnxConfig,
}

fn create_from_options(
    options: &BTreeMap<String, String>,
) -> Result<Box<dyn Transcriber>, TranscriptionError> {
    reject_unknown_options(options)?;
    let model_dir = match options
        .get("model_dir")
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        Some(model_dir) => model_dir,
        None => sole_installed_model()?,
    };
    let model_kind = match options.get("model_kind").map(String::as_str) {
        None | Some("auto") => None,
        Some(value) => Some(ModelKind::parse(value).ok_or_else(|| {
            TranscriptionError::InvalidConfiguration(format!("unknown model family '{value}'"))
        })?),
    };
    let threads = match options.get("threads") {
        Some(value) => value
            .parse::<i32>()
            .ok()
            .filter(|threads| *threads > 0)
            .ok_or_else(|| {
                TranscriptionError::InvalidConfiguration(
                    "the Sherpa ONNX thread count must be a positive integer".to_owned(),
                )
            })?,
        None => default_threads(),
    };
    Ok(Box::new(SherpaOnnxTranscriber {
        config: SherpaOnnxConfig {
            model_dir,
            model_kind,
            language: options
                .get("language")
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            threads,
        },
    }))
}

fn sole_installed_model() -> Result<String, TranscriptionError> {
    let installed = crate::transcribe::installed_models();
    let root = crate::transcribe::models_root()
        .map(|root| root.display().to_string())
        .unwrap_or_else(|| "the Voice model directory".to_owned());
    match installed.len() {
        0 => Err(TranscriptionError::ModelUnavailable(format!(
            "no speech model is installed in {root}"
        ))),
        1 => Ok(installed[0].path.to_string_lossy().into_owned()),
        _ => Err(TranscriptionError::InvalidConfiguration(format!(
            "{} models are installed in {root}; choose one in settings",
            installed.len()
        ))),
    }
}

fn default_threads() -> i32 {
    thread::available_parallelism()
        .map(|threads| i32::try_from(threads.get() / 2).unwrap_or(2))
        .unwrap_or(2)
        .max(1)
}

fn reject_unknown_options(options: &BTreeMap<String, String>) -> Result<(), TranscriptionError> {
    let accepted = ["model_dir", "model_kind", "language", "threads"];
    let Some(option) = options
        .keys()
        .find(|option| !accepted.contains(&option.as_str()))
    else {
        return Ok(());
    };
    Err(TranscriptionError::InvalidConfiguration(format!(
        "Sherpa ONNX provider does not recognize option {option}"
    )))
}

enum Command {
    Audio(AudioFrame),
    Finalize,
    Shutdown,
}

struct SherpaOnnxSession {
    commands: SyncSender<Command>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl TranscriptionSession for SherpaOnnxSession {
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

impl Drop for SherpaOnnxSession {
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

impl Transcriber for SherpaOnnxTranscriber {
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
            .name("qol-voice-sherpa-onnx".to_owned())
            .spawn(move || run_worker(config, receiver, session_started_at, events))
            .map_err(|error| TranscriptionError::ProviderUnavailable(error.to_string()))?;
        Ok(Box::new(SherpaOnnxSession {
            commands,
            worker: Mutex::new(Some(worker)),
        }))
    }
}

fn validate_format(format: AudioFormat) -> Result<(), TranscriptionError> {
    if format.sample_rate != SAMPLE_RATE {
        return Err(TranscriptionError::InvalidConfiguration(format!(
            "Sherpa ONNX requires {SAMPLE_RATE} Hz audio"
        )));
    }
    if format.channels != 1 {
        return Err(TranscriptionError::InvalidConfiguration(
            "Sherpa ONNX requires mono audio".to_owned(),
        ));
    }
    if format.encoding != AudioEncoding::PcmS16Le {
        return Err(TranscriptionError::InvalidConfiguration(
            "Sherpa ONNX requires PCM_S16LE audio".to_owned(),
        ));
    }
    Ok(())
}

fn run_worker(
    config: SherpaOnnxConfig,
    commands: mpsc::Receiver<Command>,
    session_started_at: Instant,
    events: Sender<Result<TranscriptionEvent, TranscriptionError>>,
) {
    let load_started_at = Instant::now();
    qol_runtime::probe!(
        "VOICE_SESSION",
        "event=transcriber_loading provider=sherpa-onnx model_dir={}",
        config.model_dir
    );
    let recognizer = match ModelRecognizer::load(&config) {
        Ok(recognizer) => {
            qol_runtime::probe!(
                "VOICE_SESSION",
                "event=transcriber_ready provider=sherpa-onnx family={} elapsed_ms={}",
                recognizer.kind().label(),
                elapsed_ms(load_started_at)
            );
            recognizer
        }
        Err(error) => {
            qol_runtime::probe!(
                "VOICE_SESSION",
                "event=transcriber_failed provider=sherpa-onnx elapsed_ms={} error={}",
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
                    "event=transcription_started provider=sherpa-onnx audio_samples={audio_samples}"
                );
                let result = recognizer
                    .transcribe(&audio)
                    .map(|text| TranscriptionEvent {
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
                            "event=transcription_completed provider=sherpa-onnx elapsed_ms={} characters={}",
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
                            "event=transcription_failed provider=sherpa-onnx elapsed_ms={} error={}",
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

    use super::{create_from_options, default_threads};

    type OptionCase = (&'static str, &'static [(&'static str, &'static str)], bool);

    fn options(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn a_configured_model_directory_is_taken_verbatim() {
        assert!(create_from_options(&options(&[("model_dir", "/models/parakeet")])).is_ok());
    }

    #[test]
    fn a_blank_model_directory_falls_back_to_what_is_installed() {
        let installed = crate::transcribe::installed_models().len();
        let cases: [(&str, &[(&str, &str)]); 2] =
            [("absent", &[]), ("blank", &[("model_dir", "   ")])];
        for (label, pairs) in cases {
            assert_eq!(
                create_from_options(&options(pairs)).is_ok(),
                installed == 1,
                "case: {label}"
            );
        }
    }

    #[test]
    fn provider_options_are_validated_before_any_model_is_touched() {
        let cases: [OptionCase; 5] = [
            (
                "explicit family",
                &[("model_dir", "/models/x"), ("model_kind", "nemo-ctc")],
                true,
            ),
            (
                "automatic family",
                &[("model_dir", "/models/x"), ("model_kind", "auto")],
                true,
            ),
            (
                "unknown family",
                &[("model_dir", "/models/x"), ("model_kind", "wav2vec")],
                false,
            ),
            (
                "zero threads",
                &[("model_dir", "/models/x"), ("threads", "0")],
                false,
            ),
            (
                "foreign option",
                &[("model_dir", "/models/x"), ("endpoint", "ws://host")],
                false,
            ),
        ];
        for (label, pairs, want) in cases {
            assert_eq!(
                create_from_options(&options(pairs)).is_ok(),
                want,
                "case: {label}"
            );
        }
    }

    #[test]
    fn the_default_thread_count_leaves_headroom_for_the_desktop() {
        assert!(default_threads() >= 1);
    }
}
