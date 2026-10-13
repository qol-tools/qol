use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::platform::recorder;
use crate::{Config, Rect};

pub(super) const SAMPLE_RATE: u32 = 48_000;
pub(super) const CHANNELS: u16 = 2;
const DEFAULT_DEVICE: &str = "default";

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub(super) enum AudioSource {
    Microphone(Option<String>),
    System(Option<String>),
}

#[derive(Debug, Clone, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct CapturePlan {
    pub(super) ffmpeg: PathBuf,
    pub(super) rect: Rect,
    pub(super) framerate: u32,
    pub(super) encoder: Vec<String>,
    pub(super) output: PathBuf,
    pub(super) audio: Vec<AudioSource>,
}

impl CapturePlan {
    pub(super) fn new(ffmpeg: PathBuf, rect: Rect, config: &Config, output: &Path) -> Self {
        Self {
            ffmpeg,
            rect,
            framerate: config.video.framerate.clamp(1, 240),
            encoder: recorder::encoder_args(config, true),
            output: output.to_path_buf(),
            audio: audio_sources(config),
        }
    }

    pub(super) fn ffmpeg_args(&self, audio_url: Option<&str>) -> Vec<String> {
        let mut args = ["-hide_banner", "-nostats", "-n"]
            .map(str::to_string)
            .to_vec();
        args.extend(["-thread_queue_size", "512", "-f", "gdigrab"].map(str::to_string));
        args.extend([
            "-framerate".to_string(),
            self.framerate.to_string(),
            "-offset_x".to_string(),
            self.rect.x.to_string(),
            "-offset_y".to_string(),
            self.rect.y.to_string(),
            "-video_size".to_string(),
            format!("{}x{}", self.rect.w, self.rect.h),
            "-draw_mouse".to_string(),
            "1".to_string(),
            "-i".to_string(),
            "desktop".to_string(),
        ]);
        if let Some(url) = audio_url {
            args.extend(["-thread_queue_size", "1024", "-f", "s16le"].map(str::to_string));
            args.extend([
                "-ar".to_string(),
                SAMPLE_RATE.to_string(),
                "-ac".to_string(),
                CHANNELS.to_string(),
                "-i".to_string(),
                url.to_string(),
            ]);
            args.extend(["-map", "0:v", "-map", "1:a"].map(str::to_string));
        }
        args.extend(self.encoder.iter().cloned());
        args.push(self.output.to_string_lossy().to_string());
        args
    }
}

fn audio_sources(config: &Config) -> Vec<AudioSource> {
    if !config.audio.enabled {
        return Vec::new();
    }
    let wants = |input: &str| config.audio.inputs.iter().any(|value| value == input);
    let mut sources = Vec::new();
    if wants("mic") {
        sources.push(AudioSource::Microphone(device(&config.audio.mic_device)));
    }
    if wants("system") {
        sources.push(AudioSource::System(device(&config.audio.system_device)));
    }
    sources
}

fn device(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && value != DEFAULT_DEVICE).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::{AudioSource, CapturePlan};
    use crate::{Config, Rect};
    use std::path::{Path, PathBuf};

    fn config(enabled: bool, inputs: &[&str], mic: &str, system: &str) -> Config {
        let mut config = Config::default();
        config.audio.enabled = enabled;
        config.audio.inputs = inputs.iter().map(|input| (*input).to_string()).collect();
        config.audio.mic_device = mic.to_string();
        config.audio.system_device = system.to_string();
        config
    }

    fn plan(config: &Config) -> CapturePlan {
        CapturePlan::new(
            PathBuf::from(r"C:\ffmpeg\ffmpeg.exe"),
            Rect {
                x: -1920,
                y: 40,
                w: 1280,
                h: 720,
            },
            config,
            Path::new(r"C:\Users\me\Videos\recording.mov"),
        )
    }

    #[test]
    fn audio_sources_follow_the_enabled_inputs_and_devices() {
        let mic_id = "{0.0.1.00000000}.{mic}";
        let out_id = "{0.0.0.00000000}.{out}";
        let cases = [
            (config(false, &["mic", "system"], mic_id, out_id), vec![]),
            (
                config(true, &["mic"], "default", "default"),
                vec![AudioSource::Microphone(None)],
            ),
            (
                config(true, &["system"], mic_id, out_id),
                vec![AudioSource::System(Some(out_id.to_string()))],
            ),
            (
                config(true, &["mic", "system"], mic_id, ""),
                vec![
                    AudioSource::Microphone(Some(mic_id.to_string())),
                    AudioSource::System(None),
                ],
            ),
            (config(true, &[], mic_id, out_id), vec![]),
        ];
        for (config, expected) in cases {
            assert_eq!(plan(&config).audio, expected, "{:?}", config.audio);
        }
    }

    #[test]
    fn ffmpeg_args_grab_the_physical_rect_and_mux_the_audio_feed() {
        let cases = [(None, false), (Some("tcp://127.0.0.1:50123"), true)];
        for (audio_url, muxed) in cases {
            let args = plan(&Config::default()).ffmpeg_args(audio_url);
            let has = |pair: [&str; 2]| args.windows(2).any(|window| window == pair);
            assert!(has(["-f", "gdigrab"]), "{args:?}");
            assert!(has(["-offset_x", "-1920"]), "{args:?}");
            assert!(has(["-offset_y", "40"]), "{args:?}");
            assert!(has(["-video_size", "1280x720"]), "{args:?}");
            assert!(has(["-framerate", "60"]), "{args:?}");
            assert!(has(["-i", "desktop"]), "{args:?}");
            assert!(has(["-c:v", "libx264"]), "{args:?}");
            assert!(args.contains(&"-n".to_string()), "{args:?}");
            assert!(!args.contains(&"-nostdin".to_string()), "{args:?}");
            assert_eq!(has(["-f", "s16le"]), muxed, "{args:?}");
            assert_eq!(has(["-map", "1:a"]), muxed, "{args:?}");
            if let Some(url) = audio_url {
                assert!(has(["-i", url]), "{args:?}");
            }
            assert_eq!(
                args.last().map(String::as_str),
                Some(r"C:\Users\me\Videos\recording.mov")
            );
        }
    }

    #[test]
    fn plan_round_trips_through_the_helper_request() {
        let plan = plan(&config(true, &["mic", "system"], "default", "out"));
        let json = serde_json::to_string(&plan).unwrap();
        assert_eq!(serde_json::from_str::<CapturePlan>(&json).unwrap(), plan);
    }
}
