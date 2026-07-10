use serde::{Deserialize, Serialize};

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct Config {
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub video: VideoConfig,
    #[serde(default)]
    pub capture: CaptureConfig,
    #[serde(default)]
    pub shortcuts: ShortcutsConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ShortcutsConfig {
    #[serde(default)]
    pub copy_command: CopyCommand,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CopyCommand {
    #[default]
    #[serde(alias = "platform_default")]
    CopyImage,
    CopyPath,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CaptureConfig {
    #[serde(default = "default_true")]
    #[cfg_attr(
        not(target_os = "linux"),
        allow(
            dead_code,
            reason = "window frame extents are only applied by the X11 selector"
        )
    )]
    pub include_window_frame: bool,
    #[serde(default = "default_true")]
    #[cfg_attr(
        not(any(target_os = "linux", target_os = "macos")),
        allow(
            dead_code,
            reason = "the pinned preview window only exists on linux and macos"
        )
    )]
    pub pin_border: bool,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            include_window_frame: true,
            pin_border: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AudioConfig {
    #[serde(default = "default_true")]
    #[cfg_attr(
        not(any(target_os = "linux", target_os = "macos")),
        allow(
            dead_code,
            reason = "audio capture is not implemented on this platform"
        )
    )]
    pub enabled: bool,
    #[serde(default = "default_audio_inputs")]
    #[cfg_attr(
        not(target_os = "linux"),
        allow(
            dead_code,
            reason = "explicit audio inputs are supported by linux only"
        )
    )]
    pub inputs: Vec<String>,
    #[serde(default = "default_string_default")]
    #[cfg_attr(
        not(target_os = "linux"),
        allow(
            dead_code,
            reason = "explicit audio devices are supported by linux only"
        )
    )]
    pub mic_device: String,
    #[serde(default = "default_string_default")]
    #[cfg_attr(
        not(target_os = "linux"),
        allow(
            dead_code,
            reason = "explicit audio devices are supported by linux only"
        )
    )]
    pub system_device: String,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            inputs: default_audio_inputs(),
            mic_device: default_string_default(),
            system_device: default_string_default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VideoConfig {
    #[serde(default = "default_crf")]
    #[cfg_attr(
        not(any(target_os = "linux", target_os = "macos")),
        allow(
            dead_code,
            reason = "video encoding is not implemented on this platform"
        )
    )]
    pub crf: i32,
    #[serde(default = "default_preset")]
    #[cfg_attr(
        not(any(target_os = "linux", target_os = "macos")),
        allow(
            dead_code,
            reason = "video encoding is not implemented on this platform"
        )
    )]
    pub preset: String,
    #[serde(default = "default_framerate")]
    #[cfg_attr(
        not(target_os = "linux"),
        allow(
            dead_code,
            reason = "framerate is controlled by linux ffmpeg capture only"
        )
    )]
    pub framerate: u32,
    #[serde(default = "default_format")]
    pub format: String,
}

impl Default for VideoConfig {
    fn default() -> Self {
        Self {
            crf: default_crf(),
            preset: default_preset(),
            framerate: default_framerate(),
            format: default_format(),
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_audio_inputs() -> Vec<String> {
    vec!["mic".to_string()]
}

fn default_string_default() -> String {
    "default".to_string()
}

fn default_crf() -> i32 {
    18
}

fn default_preset() -> String {
    "veryfast".to_string()
}

fn default_framerate() -> u32 {
    60
}

fn default_format() -> String {
    "mov".to_string()
}

#[cfg(test)]
fn contract_defaults() -> Config {
    qol_config::typed_defaults_from_contract(CONFIG_CONTRACT).expect("contract defaults must parse")
}

pub fn load() -> Config {
    qol_config::load_plugin_config_from_env_with_contract(crate::PLUGIN_ID, CONFIG_CONTRACT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_defaults_match_runtime_fallbacks() {
        qol_config::validate_contract_defaults_match_type::<Config>(CONFIG_CONTRACT).unwrap();

        let defaults = contract_defaults();
        assert_eq!(defaults.audio.inputs, default_audio_inputs());
        assert_eq!(defaults.audio.enabled, default_true());
        assert_eq!(defaults.audio.mic_device, default_string_default());
        assert_eq!(defaults.audio.system_device, default_string_default());
        assert_eq!(defaults.video.crf, default_crf());
        assert_eq!(defaults.video.preset, default_preset());
        assert_eq!(defaults.video.framerate, default_framerate());
        assert_eq!(defaults.video.format, default_format());
        assert_eq!(defaults.shortcuts.copy_command, CopyCommand::CopyImage);
    }
}
