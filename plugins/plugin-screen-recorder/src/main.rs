mod platform;

use anyhow::{anyhow, Context, Result};
use chrono::Local;
use serde::Deserialize;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

const PIDFILE: &str = "/tmp/record-region.pid";
const SNAP_MARGIN_PX: i32 = 50;

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct Config {
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub video: VideoConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct AudioConfig {
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

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct VideoConfig {
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

#[derive(Debug, Clone, Copy)]
pub(crate) struct Monitor {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Debug)]
struct CaptureState {
    pid: u32,
    output_file: Option<PathBuf>,
    capture_file: Option<PathBuf>,
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

fn main() -> ExitCode {
    let action = env::args().nth(1).unwrap_or_else(|| "record".to_string());
    let result = match action.as_str() {
        "record" => run_record_action(),
        "settings" => platform::open_settings(),
        _ => Err(anyhow!("Unknown action: {}", action)),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{:#}", error);
            ExitCode::from(1)
        }
    }
}

fn run_record_action() -> Result<()> {
    let config: Config = qol_config::load_plugin_config_from_env("plugin-screen-recorder");
    if let Some(state) = read_capture_state() {
        if platform::process_alive(state.pid) {
            return stop_active_recording(&state, &config);
        }
        remove_pidfile();
    }

    let mut rect = match platform::select_region()? {
        Some(region) => region,
        None => return Ok(()),
    };

    let screen_bottom = match monitor_for_selection(rect) {
        Some(monitor) => {
            rect = clamp_to_bounds(rect, monitor);
            Some(monitor.y + monitor.h)
        }
        None => {
            let virtual_monitor = platform::full_screen_bounds()?;
            rect = clamp_to_bounds(rect, virtual_monitor);
            Some(virtual_monitor.y + virtual_monitor.h)
        }
    };

    if let Some(bottom) = screen_bottom {
        let gap = bottom - (rect.y + rect.h);
        if gap > 0 && gap <= SNAP_MARGIN_PX {
            rect.h = bottom - rect.y;
        }
    }

    if rect.w <= 0 || rect.h <= 0 {
        platform::show_notification(
            "Recording failed",
            &format!("Invalid area: {}x{}", rect.w, rect.h),
            1200,
        );
        return Err(anyhow!("invalid recording area {}x{}", rect.w, rect.h));
    }

    if rect.w % 2 != 0 {
        rect.w -= 1;
    }
    if rect.h % 2 != 0 {
        rect.h -= 1;
    }

    let output_format = platform::recording_format(&config.video.format);
    let output_file = output_file_path(&output_format)?;
    let capture_file = platform::capture_file_path(&output_file);
    let pid = platform::start_capture(&rect, &config, &capture_file)?;

    write_capture_state(pid, &output_file, &capture_file)?;

    if platform::process_alive(pid) {
        platform::recording_started();
    } else {
        remove_pidfile();
        platform::show_notification(
            "Recording failed",
            &format!("Check {}", platform::CAPTURE_LOG),
            1600,
        );
        return Err(anyhow!("capture process exited immediately"));
    }

    Ok(())
}

fn stop_active_recording(state: &CaptureState, config: &Config) -> Result<()> {
    if let Err(error) = platform::stop_capture(state.pid) {
        eprintln!("failed to stop recording pid {}: {error:#}", state.pid);
        if platform::process_alive(state.pid) {
            platform::show_notification("Recording failed", "Could not stop capture process", 1800);
            return Err(error).context("capture process is still running after stop request");
        }
    }

    remove_pidfile();
    platform::recording_stopped(
        state.output_file.as_deref(),
        state.capture_file.as_deref(),
        config,
    );
    Ok(())
}

fn monitor_for_selection(rect: Rect) -> Option<Monitor> {
    let center_x = rect.x + rect.w / 2;
    let center_y = rect.y + rect.h / 2;
    let monitors = platform::get_monitors().ok()?;
    monitors.into_iter().find(|monitor| {
        center_x >= monitor.x
            && center_x < monitor.x + monitor.w
            && center_y >= monitor.y
            && center_y < monitor.y + monitor.h
    })
}

fn clamp_to_bounds(mut rect: Rect, bounds: Monitor) -> Rect {
    if rect.x < bounds.x {
        rect.w -= bounds.x - rect.x;
        rect.x = bounds.x;
    }
    if rect.y < bounds.y {
        rect.h -= bounds.y - rect.y;
        rect.y = bounds.y;
    }
    if rect.x + rect.w > bounds.x + bounds.w {
        rect.w = bounds.x + bounds.w - rect.x;
    }
    if rect.y + rect.h > bounds.y + bounds.h {
        rect.h = bounds.y + bounds.h - rect.y;
    }
    rect
}

fn read_capture_state() -> Option<CaptureState> {
    let content = fs::read_to_string(PIDFILE).ok()?;
    parse_capture_state(&content)
}

fn parse_capture_state(content: &str) -> Option<CaptureState> {
    let mut lines = content.lines();
    let pid = lines.next()?.trim().parse::<u32>().ok()?;
    let output_file = lines
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from);
    let capture_file = lines
        .next()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .or_else(|| output_file.clone());
    Some(CaptureState {
        pid,
        output_file,
        capture_file,
    })
}

fn write_capture_state(
    pid: u32,
    output_file: &std::path::Path,
    capture_file: &std::path::Path,
) -> Result<()> {
    let content = format!(
        "{}\n{}\n{}\n",
        pid,
        output_file.display(),
        capture_file.display()
    );
    fs::write(PIDFILE, content).context("failed to write pid file")
}

fn remove_pidfile() {
    let _ = fs::remove_file(PIDFILE);
}

fn output_file_path(format: &str) -> Result<PathBuf> {
    let home = env::var("HOME").context("HOME is not set")?;
    let mut videos = PathBuf::from(home);
    videos.push("Videos");
    fs::create_dir_all(&videos).context("failed to create output directory")?;
    let timestamp = Local::now().format("%F_%H-%M-%S").to_string();
    videos.push(format!("recording-{}.{}", timestamp, format));
    Ok(videos)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use qol_plugin_api::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
    }

    #[test]
    fn capture_state_reads_output_and_capture_paths() {
        let state = super::parse_capture_state("123\n/a/final.webm\n/a/native.mov\n").unwrap();
        assert_eq!(state.pid, 123);
        assert_eq!(state.output_file, Some(PathBuf::from("/a/final.webm")));
        assert_eq!(state.capture_file, Some(PathBuf::from("/a/native.mov")));
    }

    #[test]
    fn capture_state_uses_output_path_for_legacy_pidfiles() {
        let state = super::parse_capture_state("123\n/a/final.mov\n").unwrap();
        assert_eq!(state.pid, 123);
        assert_eq!(state.output_file, Some(PathBuf::from("/a/final.mov")));
        assert_eq!(state.capture_file, Some(PathBuf::from("/a/final.mov")));
    }
}
