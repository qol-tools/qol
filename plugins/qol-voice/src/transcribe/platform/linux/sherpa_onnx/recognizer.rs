use std::fs;
use std::path::{Path, PathBuf};

use sherpa_onnx::{OfflineRecognizer, OfflineRecognizerConfig};

use crate::transcribe::TranscriptionError;

use super::layout::{self, ModelFiles, ModelKind, ModelLayout};
use super::SherpaOnnxConfig;

pub(super) const SAMPLE_RATE: u32 = 16_000;

pub(super) struct ModelRecognizer {
    recognizer: OfflineRecognizer,
    kind: ModelKind,
}

impl ModelRecognizer {
    pub(super) fn load(config: &SherpaOnnxConfig) -> Result<Self, TranscriptionError> {
        let dir = PathBuf::from(&config.model_dir);
        let layout = read_layout(&dir, config.model_kind)?;
        let recognizer = OfflineRecognizer::create(&recognizer_config(&dir, &layout, config))
            .ok_or_else(|| {
                TranscriptionError::ModelUnavailable(format!(
                    "sherpa-onnx rejected the {} model in {}",
                    layout.kind.label(),
                    dir.display()
                ))
            })?;
        Ok(Self {
            recognizer,
            kind: layout.kind,
        })
    }

    pub(super) fn kind(&self) -> ModelKind {
        self.kind
    }

    pub(super) fn transcribe(&self, audio: &[f32]) -> Result<String, TranscriptionError> {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(i32::try_from(SAMPLE_RATE).unwrap_or(i32::MAX), audio);
        self.recognizer.decode(&stream);
        stream
            .get_result()
            .map(|result| result.text.trim().to_owned())
            .ok_or_else(|| {
                TranscriptionError::InferenceFailed(
                    "sherpa-onnx returned no recognition result".to_owned(),
                )
            })
    }
}

fn read_layout(dir: &Path, hint: Option<ModelKind>) -> Result<ModelLayout, TranscriptionError> {
    let entries = fs::read_dir(dir).map_err(|error| {
        TranscriptionError::ModelUnavailable(format!(
            "the model directory {} is unreadable: {error}",
            dir.display()
        ))
    })?;
    let files = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    let dir_name = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    layout::detect(dir_name, &files, hint).map_err(|error| {
        TranscriptionError::ModelUnavailable(format!("{}: {error}", dir.display()))
    })
}

fn recognizer_config(
    dir: &Path,
    layout: &ModelLayout,
    config: &SherpaOnnxConfig,
) -> OfflineRecognizerConfig {
    let path = |name: &str| Some(dir.join(name).to_string_lossy().into_owned());
    let mut recognizer = OfflineRecognizerConfig::default();
    recognizer.model_config.tokens = path(&layout.tokens);
    recognizer.model_config.num_threads = config.threads;
    match &layout.files {
        ModelFiles::Transducer {
            encoder,
            decoder,
            joiner,
        } => {
            recognizer.model_config.transducer.encoder = path(encoder);
            recognizer.model_config.transducer.decoder = path(decoder);
            recognizer.model_config.transducer.joiner = path(joiner);
            if layout.kind == ModelKind::NemoTransducer {
                recognizer.model_config.model_type = Some("nemo_transducer".to_owned());
            }
        }
        ModelFiles::EncoderDecoder { encoder, decoder } => {
            recognizer.model_config.whisper.encoder = path(encoder);
            recognizer.model_config.whisper.decoder = path(decoder);
            recognizer.model_config.whisper.language = config.language.clone();
        }
        ModelFiles::Single { model } => match layout.kind {
            ModelKind::SenseVoice => {
                recognizer.model_config.sense_voice.model = path(model);
                recognizer.model_config.sense_voice.language = config.language.clone();
                recognizer.model_config.sense_voice.use_itn = true;
            }
            ModelKind::Paraformer => recognizer.model_config.paraformer.model = path(model),
            _ => recognizer.model_config.nemo_ctc.model = path(model),
        },
        ModelFiles::Moonshine {
            preprocessor,
            encoder,
            uncached_decoder,
            cached_decoder,
        } => {
            recognizer.model_config.moonshine.preprocessor = path(preprocessor);
            recognizer.model_config.moonshine.encoder = path(encoder);
            recognizer.model_config.moonshine.uncached_decoder = path(uncached_decoder);
            recognizer.model_config.moonshine.cached_decoder = path(cached_decoder);
        }
    }
    recognizer
}

pub(super) fn append_pcm(audio: &mut Vec<f32>, pcm: &[u8]) {
    audio.extend(
        pcm.chunks_exact(2)
            .map(|sample| f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32768.0),
    );
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::layout::{ModelFiles, ModelKind, ModelLayout};
    use super::{append_pcm, recognizer_config};
    use crate::transcribe::platform::linux::sherpa_onnx::SherpaOnnxConfig;

    fn config() -> SherpaOnnxConfig {
        SherpaOnnxConfig {
            model_dir: "/models/voice".to_owned(),
            model_kind: None,
            language: Some("en".to_owned()),
            threads: 4,
        }
    }

    #[test]
    fn each_family_populates_only_its_own_model_slot() {
        let cases: [(&str, ModelLayout); 3] = [
            (
                "nemo transducer",
                ModelLayout {
                    kind: ModelKind::NemoTransducer,
                    tokens: "tokens.txt".to_owned(),
                    files: ModelFiles::Transducer {
                        encoder: "encoder.onnx".to_owned(),
                        decoder: "decoder.onnx".to_owned(),
                        joiner: "joiner.onnx".to_owned(),
                    },
                },
            ),
            (
                "whisper",
                ModelLayout {
                    kind: ModelKind::Whisper,
                    tokens: "tokens.txt".to_owned(),
                    files: ModelFiles::EncoderDecoder {
                        encoder: "encoder.onnx".to_owned(),
                        decoder: "decoder.onnx".to_owned(),
                    },
                },
            ),
            (
                "sense voice",
                ModelLayout {
                    kind: ModelKind::SenseVoice,
                    tokens: "tokens.txt".to_owned(),
                    files: ModelFiles::Single {
                        model: "model.onnx".to_owned(),
                    },
                },
            ),
        ];
        for (label, layout) in cases {
            let kind = layout.kind;
            let built = recognizer_config(Path::new("/models/voice"), &layout, &config());
            assert_eq!(
                built.model_config.tokens.as_deref(),
                Some("/models/voice/tokens.txt"),
                "case: {label}"
            );
            assert_eq!(built.model_config.num_threads, 4, "case: {label}");
            assert_eq!(
                built.model_config.transducer.joiner.is_some(),
                kind == ModelKind::NemoTransducer,
                "case: {label}"
            );
            assert_eq!(
                built.model_config.whisper.encoder.is_some(),
                kind == ModelKind::Whisper,
                "case: {label}"
            );
            assert_eq!(
                built.model_config.sense_voice.model.is_some(),
                kind == ModelKind::SenseVoice,
                "case: {label}"
            );
        }
    }

    #[test]
    fn nemo_transducers_declare_their_model_type() {
        let layout = ModelLayout {
            kind: ModelKind::NemoTransducer,
            tokens: "tokens.txt".to_owned(),
            files: ModelFiles::Transducer {
                encoder: "encoder.onnx".to_owned(),
                decoder: "decoder.onnx".to_owned(),
                joiner: "joiner.onnx".to_owned(),
            },
        };
        let built = recognizer_config(Path::new("/models/voice"), &layout, &config());
        assert_eq!(
            built.model_config.model_type.as_deref(),
            Some("nemo_transducer")
        );
    }

    #[test]
    fn pcm_samples_become_normalized_audio() {
        let mut audio = Vec::new();
        append_pcm(&mut audio, &[0x00, 0x00, 0x00, 0x40, 0x00, 0xc0]);
        assert_eq!(audio, vec![0.0, 0.5, -0.5]);
    }
}
