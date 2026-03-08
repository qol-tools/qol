mod platform;

use anyhow::{anyhow, Context, Result};
use chrono::Local;
use serde::Deserialize;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

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
    pub enabled: bool,
    #[serde(default = "default_audio_inputs")]
    pub inputs: Vec<String>,
    #[serde(default = "default_string_default")]
    pub mic_device: String,
    #[serde(default = "default_string_default")]
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
    pub crf: i32,
    #[serde(default = "default_preset")]
    pub preset: String,
    #[serde(default = "default_framerate")]
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
    "mkv".to_string()
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
    if let Some(pid) = read_pid() {
        if platform::process_alive(pid) {
            platform::stop_capture(pid)?;
            thread::sleep(Duration::from_millis(250));
            remove_pidfile();
            platform::show_notification("Recording stopped", "Saved to ~/Videos", 2000);
            return Ok(());
        }
        remove_pidfile();
    }

    let config = load_config(plugin_dir().join("config.json"));
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

    let output_file = output_file_path(&config.video.format)?;
    let pid = platform::start_capture(&rect, &config, &output_file)?;

    fs::write(PIDFILE, pid.to_string()).context("failed to write pid file")?;
    thread::sleep(Duration::from_millis(500));

    if platform::process_alive(pid) {
        platform::show_notification("Recording started", "Press your hotkey to stop", 1200);
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

fn plugin_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn load_config(path: PathBuf) -> Config {
    let Ok(content) = fs::read_to_string(path) else {
        return Config::default();
    };
    serde_json::from_str::<Config>(&content).unwrap_or_default()
}

fn read_pid() -> Option<u32> {
    let content = fs::read_to_string(PIDFILE).ok()?;
    content.trim().parse::<u32>().ok()
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
    use qol_tray::plugins::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        let manifest_str =
            std::fs::read_to_string("plugin.toml").expect("Failed to read plugin.toml");
        let manifest: PluginManifest =
            toml::from_str(&manifest_str).expect("Failed to parse plugin.toml");
        manifest.validate().expect("Manifest validation failed");
    }
}
