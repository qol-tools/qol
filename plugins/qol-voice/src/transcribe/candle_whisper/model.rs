use std::fs;
use std::path::{Path, PathBuf};

use candle_core::{Device, IndexOp, Tensor};
use candle_nn::{ops::softmax, VarBuilder};
use candle_transformers::models::whisper::{self, audio, Config};
use hf_hub::api::sync::{Api, ApiBuilder, ApiRepo};
use hf_hub::{Repo, RepoType};
use tokenizers::Tokenizer;

use super::CandleWhisperConfig;
use crate::transcribe::TranscriptionError;

pub(super) const DEFAULT_MODEL_ID: &str = "openai/whisper-tiny.en";
pub(super) const DEFAULT_REVISION: &str = "refs/pr/15";
pub(super) const SAMPLE_RATE: u32 = whisper::SAMPLE_RATE as u32;
const MEL_FILTER_MODEL_ID: &str = "lmz/candle-whisper";

pub(super) fn append_pcm(audio: &mut Vec<f32>, pcm: &[u8]) {
    audio.extend(
        pcm.chunks_exact(2)
            .map(|sample| f32::from(i16::from_le_bytes([sample[0], sample[1]])) / 32768.0),
    );
    if audio.len() <= whisper::N_SAMPLES {
        return;
    }
    audio.drain(..audio.len() - whisper::N_SAMPLES);
}

pub(super) struct WhisperDecoder {
    model: whisper::model::Whisper,
    tokenizer: Tokenizer,
    device: Device,
    mel_filters: Vec<f32>,
    suppress_tokens: Tensor,
    initial_tokens: Vec<u32>,
    eot_token: u32,
    no_speech_token: Option<u32>,
}

impl WhisperDecoder {
    pub(super) fn load(config: &CandleWhisperConfig) -> Result<Self, TranscriptionError> {
        if config
            .model_id
            .split_once('/')
            .is_none_or(|(owner, name)| owner.is_empty() || name.is_empty())
        {
            return Err(TranscriptionError::InvalidConfiguration(
                "model_id must use owner/name form".to_owned(),
            ));
        }
        let client = ApiBuilder::from_env()
            .with_progress(false)
            .build()
            .map_err(model_error)?;
        let repository = client.repo(Repo::with_revision(
            config.model_id.clone(),
            RepoType::Model,
            config.revision.clone(),
        ));
        let model_config_path = repository.get("config.json").map_err(model_error)?;
        let tokenizer_path = repository.get("tokenizer.json").map_err(model_error)?;
        let weights_path = repository.get("model.safetensors").map_err(model_error)?;
        let mel_filters_path = download_mel_filters(&client)?;
        let model_config = fs::read_to_string(model_config_path).map_err(model_error)?;
        let model_config = serde_json::from_str::<Config>(&model_config).map_err(model_error)?;
        let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(model_error)?;
        let device = Device::Cpu;
        let mel_filters = load_mel_filters(&mel_filters_path, model_config.num_mel_bins, &device)?;
        let weights = fs::read(weights_path).map_err(model_error)?;
        let variables = VarBuilder::from_buffered_safetensors(weights, whisper::DTYPE, &device)
            .map_err(model_error)?;
        let model = whisper::model::Whisper::load(&variables, model_config).map_err(model_error)?;
        let initial_tokens = initial_tokens(&tokenizer, config.language.as_deref())?;
        let eot_token = token_id(&tokenizer, whisper::EOT_TOKEN)?;
        let no_speech_token = whisper::NO_SPEECH_TOKENS
            .iter()
            .find_map(|token| tokenizer.token_to_id(token));
        let suppress_tokens = suppression_tensor(&model, &device)?;
        Ok(Self {
            model,
            tokenizer,
            device,
            mel_filters,
            suppress_tokens,
            initial_tokens,
            eot_token,
            no_speech_token,
        })
    }

    pub(super) fn transcribe(&mut self, pcm: &[f32]) -> Result<String, TranscriptionError> {
        let mel = audio::pcm_to_mel(&self.model.config, pcm, &self.mel_filters);
        let mel_columns = mel.len() / self.model.config.num_mel_bins;
        let mel = Tensor::from_vec(
            mel,
            (1, self.model.config.num_mel_bins, mel_columns),
            &self.device,
        )
        .and_then(|mel| mel.narrow(2, 0, mel_columns.min(whisper::N_FRAMES)))
        .map_err(inference_error)?;
        self.decode(&mel)
    }

    fn decode(&mut self, mel: &Tensor) -> Result<String, TranscriptionError> {
        self.model.reset_kv_cache();
        let audio_features = self
            .model
            .encoder
            .forward(mel, true)
            .map_err(inference_error)?;
        let mut tokens = self.initial_tokens.clone();
        let mut no_speech_probability = None;
        let sample_length = self.model.config.max_target_positions / 2;
        for index in 0..sample_length {
            let token_tensor = Tensor::new(tokens.as_slice(), mel.device())
                .and_then(|tokens| tokens.unsqueeze(0))
                .map_err(inference_error)?;
            let decoded = self
                .model
                .decoder
                .forward(&token_tensor, &audio_features, index == 0)
                .map_err(inference_error)?;
            if index == 0 {
                no_speech_probability = self
                    .no_speech_probability(&decoded)
                    .map_err(inference_error)?;
            }
            let (_, sequence_length, _) = decoded.dims3().map_err(inference_error)?;
            let logits = self
                .model
                .decoder
                .final_linear(
                    &decoded
                        .i((..1, sequence_length - 1..))
                        .map_err(inference_error)?,
                )
                .and_then(|logits| logits.i(0))
                .and_then(|logits| logits.i(0))
                .and_then(|logits| logits.broadcast_add(&self.suppress_tokens))
                .map_err(inference_error)?;
            let next_token = greedy_token(&logits)?;
            if next_token == self.eot_token {
                break;
            }
            tokens.push(next_token);
        }
        if no_speech_probability
            .is_some_and(|probability| probability > whisper::NO_SPEECH_THRESHOLD)
        {
            return Ok(String::new());
        }
        self.tokenizer
            .decode(&tokens, true)
            .map(|text| text.trim().to_owned())
            .map_err(inference_error)
    }

    fn no_speech_probability(&self, decoded: &Tensor) -> candle_core::Result<Option<f64>> {
        let Some(no_speech_token) = self.no_speech_token else {
            return Ok(None);
        };
        let logits = self.model.decoder.final_linear(&decoded.i(..1)?)?;
        let logits = logits.i(0)?.i(0)?;
        let probability = softmax(&logits, 0)?
            .i(no_speech_token as usize)?
            .to_scalar::<f32>()?;
        Ok(Some(f64::from(probability)))
    }
}

fn initial_tokens(
    tokenizer: &Tokenizer,
    language: Option<&str>,
) -> Result<Vec<u32>, TranscriptionError> {
    let mut tokens = vec![token_id(tokenizer, whisper::SOT_TOKEN)?];
    if let Some(language) = language {
        tokens.push(token_id(tokenizer, &format!("<|{language}|>"))?);
    }
    tokens.push(token_id(tokenizer, whisper::TRANSCRIBE_TOKEN)?);
    tokens.push(token_id(tokenizer, whisper::NO_TIMESTAMPS_TOKEN)?);
    Ok(tokens)
}

fn token_id(tokenizer: &Tokenizer, token: &str) -> Result<u32, TranscriptionError> {
    tokenizer.token_to_id(token).ok_or_else(|| {
        TranscriptionError::ModelUnavailable(format!("model tokenizer has no token {token}"))
    })
}

fn suppression_tensor(
    model: &whisper::model::Whisper,
    device: &Device,
) -> Result<Tensor, TranscriptionError> {
    let values = (0..model.config.vocab_size as u32)
        .map(|token| {
            if model.config.suppress_tokens.contains(&token) {
                return f32::NEG_INFINITY;
            }
            0.0
        })
        .collect::<Vec<_>>();
    Tensor::new(values.as_slice(), device).map_err(model_error)
}

fn greedy_token(logits: &Tensor) -> Result<u32, TranscriptionError> {
    let values = logits.to_vec1::<f32>().map_err(inference_error)?;
    values
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(index, _)| index as u32)
        .ok_or_else(|| TranscriptionError::InferenceFailed("model returned no logits".to_owned()))
}

fn download_mel_filters(client: &Api) -> Result<PathBuf, TranscriptionError> {
    let repository: ApiRepo = client.repo(Repo::space(MEL_FILTER_MODEL_ID.to_owned()));
    repository
        .get("mel_filters.safetensors")
        .map_err(model_error)
}

fn load_mel_filters(
    path: &Path,
    count: usize,
    device: &Device,
) -> Result<Vec<f32>, TranscriptionError> {
    let tensors = candle_core::safetensors::load(path, device).map_err(model_error)?;
    let name = format!("mel_{count}");
    let tensor = tensors.get(&name).ok_or_else(|| {
        TranscriptionError::ModelUnavailable(format!("mel filter asset does not contain {name}"))
    })?;
    tensor
        .flatten_all()
        .and_then(|tensor| tensor.to_vec1::<f32>())
        .map_err(model_error)
}

fn model_error(error: impl std::fmt::Display) -> TranscriptionError {
    TranscriptionError::ModelUnavailable(error.to_string())
}

fn inference_error(error: impl std::fmt::Display) -> TranscriptionError {
    TranscriptionError::InferenceFailed(error.to_string())
}
