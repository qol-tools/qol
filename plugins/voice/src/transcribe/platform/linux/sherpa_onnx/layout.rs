#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ModelKind {
    Transducer,
    NemoTransducer,
    Whisper,
    SenseVoice,
    Paraformer,
    NemoCtc,
    Moonshine,
}

impl ModelKind {
    pub(super) fn parse(value: &str) -> Option<Self> {
        match value {
            "transducer" => Some(Self::Transducer),
            "nemo-transducer" => Some(Self::NemoTransducer),
            "whisper" => Some(Self::Whisper),
            "sense-voice" => Some(Self::SenseVoice),
            "paraformer" => Some(Self::Paraformer),
            "nemo-ctc" => Some(Self::NemoCtc),
            "moonshine" => Some(Self::Moonshine),
            _ => None,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Transducer => "transducer",
            Self::NemoTransducer => "nemo-transducer",
            Self::Whisper => "whisper",
            Self::SenseVoice => "sense-voice",
            Self::Paraformer => "paraformer",
            Self::NemoCtc => "nemo-ctc",
            Self::Moonshine => "moonshine",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ModelFiles {
    Transducer {
        encoder: String,
        decoder: String,
        joiner: String,
    },
    EncoderDecoder {
        encoder: String,
        decoder: String,
    },
    Single {
        model: String,
    },
    Moonshine {
        preprocessor: String,
        encoder: String,
        uncached_decoder: String,
        cached_decoder: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ModelLayout {
    pub(super) kind: ModelKind,
    pub(super) tokens: String,
    pub(super) files: ModelFiles,
}

pub(super) fn detect(
    dir_name: &str,
    files: &[String],
    hint: Option<ModelKind>,
) -> Result<ModelLayout, String> {
    let tokens = pick(files, "tokens.txt")
        .ok_or_else(|| "the model directory has no tokens.txt".to_owned())?;
    let kind = match hint {
        Some(kind) => kind,
        None => infer_kind(dir_name, files)?,
    };
    Ok(ModelLayout {
        kind,
        tokens,
        files: gather(kind, files)?,
    })
}

fn infer_kind(dir_name: &str, files: &[String]) -> Result<ModelKind, String> {
    let dir_name = dir_name.to_ascii_lowercase();
    if pick(files, "joiner").is_some() {
        return Ok(
            if dir_name.contains("nemo") || dir_name.contains("parakeet") {
                ModelKind::NemoTransducer
            } else {
                ModelKind::Transducer
            },
        );
    }
    if pick(files, "preprocess").is_some() {
        return Ok(ModelKind::Moonshine);
    }
    if pick(files, "encoder").is_some() || pick(files, "encode").is_some() {
        return Ok(ModelKind::Whisper);
    }
    if dir_name.contains("sense") {
        return Ok(ModelKind::SenseVoice);
    }
    if dir_name.contains("paraformer") {
        return Ok(ModelKind::Paraformer);
    }
    if dir_name.contains("nemo") || dir_name.contains("ctc") {
        return Ok(ModelKind::NemoCtc);
    }
    Err(
        "the model family cannot be inferred from this directory; set the model family explicitly"
            .to_owned(),
    )
}

fn gather(kind: ModelKind, files: &[String]) -> Result<ModelFiles, String> {
    match kind {
        ModelKind::Transducer | ModelKind::NemoTransducer => Ok(ModelFiles::Transducer {
            encoder: require(files, "encoder")?,
            decoder: require(files, "decoder")?,
            joiner: require(files, "joiner")?,
        }),
        ModelKind::Whisper => Ok(ModelFiles::EncoderDecoder {
            encoder: require(files, "encoder")?,
            decoder: require(files, "decoder")?,
        }),
        ModelKind::SenseVoice | ModelKind::Paraformer | ModelKind::NemoCtc => {
            Ok(ModelFiles::Single {
                model: require(files, "model")?,
            })
        }
        ModelKind::Moonshine => Ok(ModelFiles::Moonshine {
            preprocessor: require(files, "preprocess")?,
            encoder: require(files, "encode")?,
            uncached_decoder: require(files, "uncached_decode")?,
            cached_decoder: require(files, "cached_decode")?,
        }),
    }
}

fn require(files: &[String], marker: &str) -> Result<String, String> {
    pick(files, marker).ok_or_else(|| format!("the model directory has no {marker} file"))
}

fn pick(files: &[String], marker: &str) -> Option<String> {
    let mut matches = files
        .iter()
        .filter(|name| matches_marker(name, marker))
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        (left.contains(".int8."), left.len(), left.as_str()).cmp(&(
            right.contains(".int8."),
            right.len(),
            right.as_str(),
        ))
    });
    matches.first().map(|name| (*name).clone())
}

fn matches_marker(name: &str, marker: &str) -> bool {
    let name = name.to_ascii_lowercase();
    if marker == "tokens.txt" {
        return name.ends_with("tokens.txt");
    }
    if !name.ends_with(".onnx") {
        return false;
    }
    match marker {
        "encode" => name.contains("encode") && !name.contains("encoder"),
        "decoder" => name.contains("decoder") && !name.contains("uncached"),
        "cached_decode" => name.contains("cached_decode") && !name.contains("uncached"),
        "model" => name.contains("model"),
        _ => name.contains(marker),
    }
}

#[cfg(test)]
mod tests {
    use super::{detect, ModelFiles, ModelKind};

    fn names(files: &[&str]) -> Vec<String> {
        files.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn released_model_directories_resolve_to_their_family() {
        let cases: [(&str, &[&str], ModelKind); 6] = [
            (
                "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
                &[
                    "encoder.int8.onnx",
                    "decoder.int8.onnx",
                    "joiner.int8.onnx",
                    "tokens.txt",
                ],
                ModelKind::NemoTransducer,
            ),
            (
                "sherpa-onnx-zipformer-en-2023-06-26",
                &[
                    "encoder-epoch-99-avg-1.onnx",
                    "decoder-epoch-99-avg-1.onnx",
                    "joiner-epoch-99-avg-1.onnx",
                    "tokens.txt",
                ],
                ModelKind::Transducer,
            ),
            (
                "sherpa-onnx-whisper-large-v3-turbo",
                &[
                    "large-v3-turbo-encoder.onnx",
                    "large-v3-turbo-decoder.onnx",
                    "large-v3-turbo-tokens.txt",
                ],
                ModelKind::Whisper,
            ),
            (
                "sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17",
                &["model.int8.onnx", "tokens.txt"],
                ModelKind::SenseVoice,
            ),
            (
                "sherpa-onnx-paraformer-zh-2023-09-14",
                &["model.int8.onnx", "tokens.txt"],
                ModelKind::Paraformer,
            ),
            (
                "sherpa-onnx-moonshine-tiny-en-int8",
                &[
                    "preprocess.onnx",
                    "encode.int8.onnx",
                    "uncached_decode.int8.onnx",
                    "cached_decode.int8.onnx",
                    "tokens.txt",
                ],
                ModelKind::Moonshine,
            ),
        ];
        for (dir_name, files, want) in cases {
            let layout = detect(dir_name, &names(files), None)
                .unwrap_or_else(|error| panic!("case {dir_name}: {error}"));
            assert_eq!(layout.kind, want, "case: {dir_name}");
        }
    }

    #[test]
    fn transducer_layouts_carry_every_model_file() {
        let layout = detect(
            "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
            &names(&[
                "encoder.int8.onnx",
                "decoder.int8.onnx",
                "joiner.int8.onnx",
                "tokens.txt",
            ]),
            None,
        )
        .unwrap();
        assert_eq!(layout.tokens, "tokens.txt");
        assert_eq!(
            layout.files,
            ModelFiles::Transducer {
                encoder: "encoder.int8.onnx".to_owned(),
                decoder: "decoder.int8.onnx".to_owned(),
                joiner: "joiner.int8.onnx".to_owned(),
            }
        );
    }

    #[test]
    fn full_precision_weights_win_over_quantized_ones() {
        let layout = detect(
            "sherpa-onnx-sense-voice",
            &names(&["model.onnx", "model.int8.onnx", "tokens.txt"]),
            None,
        )
        .unwrap();
        assert_eq!(
            layout.files,
            ModelFiles::Single {
                model: "model.onnx".to_owned(),
            }
        );
    }

    #[test]
    fn an_explicit_family_overrides_inference() {
        let layout = detect(
            "my-model",
            &names(&["model.onnx", "tokens.txt"]),
            Some(ModelKind::NemoCtc),
        )
        .unwrap();
        assert_eq!(layout.kind, ModelKind::NemoCtc);
    }

    #[test]
    fn unusable_directories_fail_with_the_reason() {
        let cases: [(&str, &[&str]); 3] = [
            ("no tokens", &["model.onnx"]),
            ("unknown family", &["model.onnx", "tokens.txt"]),
            ("missing joiner", &["encoder.onnx", "tokens.txt"]),
        ];
        for (label, files) in cases {
            let detected = detect("mystery-model", &names(files), None);
            assert!(detected.is_err(), "case: {label}");
        }
    }
}
