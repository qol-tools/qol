use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::listen::{AudioInputRequest, ListenConfig};
use crate::transcribe::TranscriberRequest;

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub activation: ActivationConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub recognition: RecognitionConfig,
    #[serde(default)]
    pub routing: RoutingConfig,
    #[serde(default)]
    pub segmentation: SegmentationConfig,
}

impl Config {
    pub fn input_request(&self) -> AudioInputRequest {
        let device_id =
            (self.audio.input_device != "default").then(|| self.audio.input_device.clone());
        AudioInputRequest { device_id }
    }

    pub fn listen_config(&self) -> ListenConfig {
        ListenConfig {
            threshold_permille: self.segmentation.threshold_permille,
            onset_ms: self.segmentation.onset_ms,
            silence_ms: self.segmentation.silence_ms,
            pre_roll_ms: self.segmentation.pre_roll_ms,
            max_utterance_ms: self.segmentation.max_utterance_ms,
        }
    }

    pub fn transcriber_request(&self) -> Option<TranscriberRequest> {
        self.recognition.enabled.then(|| {
            let options = match self.recognition.provider.as_str() {
                "websocket" => provider_options(&[
                    ("endpoint", self.recognition.websocket_endpoint.as_str()),
                    ("engine", self.recognition.websocket_engine.as_str()),
                ]),
                "sherpa-onnx" => provider_options(&[
                    ("model_dir", self.recognition.model_dir.as_str()),
                    ("model_kind", self.recognition.model_kind.as_str()),
                ]),
                _ => BTreeMap::new(),
            };
            TranscriberRequest {
                provider: self.recognition.provider.clone(),
                options,
            }
        })
    }
}

fn provider_options(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .filter(|(_, value)| !value.trim().is_empty())
        .map(|(key, value)| ((*key).to_owned(), (*value).trim().to_owned()))
        .collect()
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ActivationConfig {
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AudioConfig {
    #[serde(default = "default_input_device")]
    pub input_device: String,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input_device: default_input_device(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RecognitionConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default)]
    pub websocket_endpoint: String,
    #[serde(default)]
    pub websocket_engine: String,
    #[serde(default)]
    pub model_dir: String,
    #[serde(default = "default_model_kind")]
    pub model_kind: String,
}

impl Default for RecognitionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            provider: default_provider(),
            websocket_endpoint: String::new(),
            websocket_engine: String::new(),
            model_dir: String::new(),
            model_kind: default_model_kind(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RoutingConfig {
    #[serde(default = "default_target")]
    pub target: String,
    #[serde(default)]
    pub delivery_mode: qol_terminal_sessions::DeliveryMode,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            target: default_target(),
            delivery_mode: qol_terminal_sessions::DeliveryMode::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct SegmentationConfig {
    #[serde(default = "default_threshold")]
    pub threshold_permille: u16,
    #[serde(default = "default_onset")]
    pub onset_ms: u64,
    #[serde(default = "default_silence")]
    pub silence_ms: u64,
    #[serde(default = "default_pre_roll")]
    pub pre_roll_ms: u64,
    #[serde(default = "default_max_utterance")]
    pub max_utterance_ms: u64,
}

impl Default for SegmentationConfig {
    fn default() -> Self {
        Self {
            threshold_permille: default_threshold(),
            onset_ms: default_onset(),
            silence_ms: default_silence(),
            pre_roll_ms: default_pre_roll(),
            max_utterance_ms: default_max_utterance(),
        }
    }
}

pub fn load() -> Config {
    qol_config::load_plugin_config_from_env_with_contract(crate::PLUGIN_ID, CONFIG_CONTRACT)
}

pub fn inspect(
) -> Result<qol_config::PluginConfigInspection<Config>, qol_config::PluginConfigInspectionError> {
    qol_config::inspect_plugin_config_from_env_with_contract(crate::PLUGIN_ID, CONFIG_CONTRACT)
}

fn default_true() -> bool {
    true
}

fn default_input_device() -> String {
    "default".to_owned()
}

fn default_provider() -> String {
    "auto".to_owned()
}

fn default_model_kind() -> String {
    "auto".to_owned()
}

fn default_target() -> String {
    "none".to_owned()
}

fn default_threshold() -> u16 {
    10
}

fn default_onset() -> u64 {
    100
}

fn default_silence() -> u64 {
    700
}

fn default_pre_roll() -> u64 {
    300
}

fn default_max_utterance() -> u64 {
    30_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_defaults_match_runtime_config() {
        qol_config::validate_contract_defaults_match_type::<Config>(CONFIG_CONTRACT).unwrap();
        let defaults = qol_config::typed_defaults_from_contract::<Config>(CONFIG_CONTRACT).unwrap();

        assert_eq!(defaults.audio.input_device, "default");
        assert!(!defaults.activation.enabled);
        assert!(defaults.recognition.enabled);
        assert_eq!(defaults.recognition.provider, "auto");
        assert_eq!(defaults.routing.target, "none");
        assert_eq!(
            defaults.routing.delivery_mode,
            qol_terminal_sessions::DeliveryMode::Insert
        );
        assert_eq!(defaults.listen_config(), ListenConfig::default());
    }
}
