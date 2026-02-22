use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::env;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::thread;
use std::time::Duration;

const PIDFILE: &str = "/tmp/record-region.pid";
const LOGFILE: &str = "/tmp/record-region.log";
const SNAP_MARGIN_PX: i32 = 50;
const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-screen-recorder/";

#[derive(Debug, Clone, Deserialize)]
struct Config {
    #[serde(default)]
    audio: AudioConfig,
    #[serde(default)]
    video: VideoConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            audio: AudioConfig::default(),
            video: VideoConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct AudioConfig {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_audio_inputs")]
    inputs: Vec<String>,
    #[serde(default = "default_string_default")]
    mic_device: String,
    #[serde(default = "default_string_default")]
    system_device: String,
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
struct VideoConfig {
    #[serde(default = "default_crf")]
    crf: i32,
    #[serde(default = "default_preset")]
    preset: String,
    #[serde(default = "default_framerate")]
    framerate: u32,
    #[serde(default = "default_format")]
    format: String,
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
struct Monitor {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
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
        "audio-settings" => open_audio_settings(),
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
        if process_exists(pid) {
            stop_recording(pid)?;
            return Ok(());
        }
        remove_pidfile();
    }

    let config = load_config(plugin_dir().join("config.json"));
    let mut rect = match select_region()? {
        Some(region) => region,
        None => return Ok(()),
    };

    let screen_bottom = match monitor_for_selection(rect) {
        Some(monitor) => {
            rect = clamp_to_bounds(rect, monitor);
            Some(monitor.y + monitor.h)
        }
        None => {
            let virtual_monitor = full_screen_monitor()?;
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
        show_notification(
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
    start_recording(rect, &config, &output_file)?;
    Ok(())
}

fn open_audio_settings() -> Result<()> {
    Command::new("xdg-open")
        .arg(SETTINGS_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to open settings URL")?;
    Ok(())
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

fn process_exists(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

fn stop_recording(pid: u32) -> Result<()> {
    Command::new("kill")
        .args(["-INT", &pid.to_string()])
        .status()
        .context("failed to send SIGINT to ffmpeg")?;
    thread::sleep(Duration::from_millis(250));
    remove_pidfile();
    show_notification("Recording stopped", "Saved to ~/Videos", 2000);
    Ok(())
}

fn remove_pidfile() {
    let _ = fs::remove_file(PIDFILE);
}

fn select_region() -> Result<Option<Rect>> {
    let output = Command::new("slop")
        .args([
            "--highlight",
            "--color=1,0,0,0.65",
            "-b",
            "0",
            "-f",
            "%x,%y,%w,%h",
        ])
        .output()
        .context("failed to run slop")?;

    if !output.status.success() {
        return Ok(None);
    }

    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return Ok(None);
    }

    parse_selection_geometry(&raw).map(Some)
}

fn parse_selection_geometry(raw: &str) -> Result<Rect> {
    let values: Vec<i32> = raw
        .split(',')
        .map(str::trim)
        .map(str::parse::<i32>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("invalid selection geometry")?;
    if values.len() != 4 {
        return Err(anyhow!(
            "expected 4 values in geometry, got {}",
            values.len()
        ));
    }
    Ok(Rect {
        x: values[0],
        y: values[1],
        w: values[2],
        h: values[3],
    })
}

fn monitor_for_selection(rect: Rect) -> Option<Monitor> {
    let center_x = rect.x + rect.w / 2;
    let center_y = rect.y + rect.h / 2;
    let monitors = xrandr_monitors().ok()?;
    monitors.into_iter().find(|monitor| {
        center_x >= monitor.x
            && center_x < monitor.x + monitor.w
            && center_y >= monitor.y
            && center_y < monitor.y + monitor.h
    })
}

fn xrandr_monitors() -> Result<Vec<Monitor>> {
    let output = Command::new("xrandr")
        .args(["--query"])
        .output()
        .context("failed to run xrandr")?;
    if !output.status.success() {
        return Err(anyhow!("xrandr failed"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let monitors: Vec<Monitor> = stdout.lines().filter_map(parse_xrandr_line).collect();
    if monitors.is_empty() {
        return Err(anyhow!("no monitors found from xrandr"));
    }
    Ok(monitors)
}

fn parse_xrandr_line(line: &str) -> Option<Monitor> {
    if !line.contains(" connected") {
        return None;
    }
    let mut parts = line.split_whitespace();
    let _name = parts.next()?;
    let geometry = line
        .split_whitespace()
        .find(|token| token.contains('x') && token.contains('+'))?;
    parse_monitor_geometry(geometry)
}

fn parse_monitor_geometry(token: &str) -> Option<Monitor> {
    let x_split = token.find('x')?;
    let width = token[..x_split].parse::<i32>().ok()?;
    let after_x = &token[x_split + 1..];
    let first_sign = after_x.find(['+', '-'])?;
    let height = after_x[..first_sign].parse::<i32>().ok()?;
    let after_height = &after_x[first_sign..];
    let second_sign = after_height[1..].find(['+', '-'])? + 1;
    let x = after_height[..second_sign].parse::<i32>().ok()?;
    let y = after_height[second_sign..].parse::<i32>().ok()?;
    Some(Monitor {
        x,
        y,
        w: width,
        h: height,
    })
}

fn full_screen_monitor() -> Result<Monitor> {
    let output = Command::new("xdpyinfo")
        .output()
        .context("failed to run xdpyinfo")?;
    if !output.status.success() {
        return Err(anyhow!("xdpyinfo failed"));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let dimensions = stdout
        .lines()
        .find_map(|line| {
            if !line.contains("dimensions:") {
                return None;
            }
            line.split_whitespace().find(|token| {
                token.contains('x') && token.chars().all(|c| c.is_ascii_digit() || c == 'x')
            })
        })
        .ok_or_else(|| anyhow!("could not read dimensions from xdpyinfo"))?;
    let split = dimensions
        .find('x')
        .ok_or_else(|| anyhow!("invalid dimensions"))?;
    let w = dimensions[..split]
        .parse::<i32>()
        .context("invalid width from xdpyinfo")?;
    let h = dimensions[split + 1..]
        .parse::<i32>()
        .context("invalid height from xdpyinfo")?;
    Ok(Monitor { x: 0, y: 0, w, h })
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

fn output_file_path(format: &str) -> Result<PathBuf> {
    let home = env::var("HOME").context("HOME is not set")?;
    let mut videos = PathBuf::from(home);
    videos.push("Videos");
    fs::create_dir_all(&videos).context("failed to create output directory")?;
    let stamp = Command::new("date")
        .arg("+%F_%H-%M-%S")
        .output()
        .context("failed to generate timestamp")?;
    if !stamp.status.success() {
        return Err(anyhow!("date command failed"));
    }
    let timestamp = String::from_utf8_lossy(&stamp.stdout).trim().to_string();
    videos.push(format!("recording-{}.{}", timestamp, format));
    Ok(videos)
}

fn start_recording(rect: Rect, config: &Config, output_file: &Path) -> Result<()> {
    let mut args = vec![
        "-f".to_string(),
        "x11grab".to_string(),
        "-video_size".to_string(),
        format!("{}x{}", rect.w, rect.h),
        "-framerate".to_string(),
        config.video.framerate.to_string(),
        "-i".to_string(),
        format!(":0.0+{},{}", rect.x, rect.y),
    ];

    if config.audio.enabled {
        let has_mic = config.audio.inputs.iter().any(|input| input == "mic");
        let has_system = config.audio.inputs.iter().any(|input| input == "system");
        if has_mic && has_system {
            args.extend_from_slice(&[
                "-f".to_string(),
                "pulse".to_string(),
                "-i".to_string(),
                config.audio.mic_device.clone(),
                "-f".to_string(),
                "pulse".to_string(),
                "-i".to_string(),
                format!("{}.monitor", config.audio.system_device),
                "-filter_complex".to_string(),
                "[1:a][2:a]amerge=inputs=2[aout]".to_string(),
                "-map".to_string(),
                "0:v".to_string(),
                "-map".to_string(),
                "[aout]".to_string(),
                "-c:a".to_string(),
                "aac".to_string(),
                "-b:a".to_string(),
                "192k".to_string(),
            ]);
        } else if has_mic {
            args.extend_from_slice(&[
                "-f".to_string(),
                "pulse".to_string(),
                "-i".to_string(),
                config.audio.mic_device.clone(),
                "-c:a".to_string(),
                "aac".to_string(),
                "-b:a".to_string(),
                "192k".to_string(),
            ]);
        } else if has_system {
            args.extend_from_slice(&[
                "-f".to_string(),
                "pulse".to_string(),
                "-i".to_string(),
                format!("{}.monitor", config.audio.system_device),
                "-c:a".to_string(),
                "aac".to_string(),
                "-b:a".to_string(),
                "192k".to_string(),
            ]);
        }
    }

    args.extend_from_slice(&[
        "-c:v".to_string(),
        "libx264".to_string(),
        "-crf".to_string(),
        config.video.crf.to_string(),
        "-preset".to_string(),
        config.video.preset.clone(),
        "-pix_fmt".to_string(),
        "yuv420p".to_string(),
        output_file.to_string_lossy().to_string(),
    ]);

    let log_file = File::create(LOGFILE).context("failed to create recording log file")?;
    let stdout_log = log_file
        .try_clone()
        .context("failed to clone recording log file")?;

    let child = Command::new("ffmpeg")
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_log))
        .stderr(Stdio::from(log_file))
        .spawn()
        .context("failed to start ffmpeg")?;

    let pid = child.id();
    fs::write(PIDFILE, pid.to_string()).context("failed to write pid file")?;
    thread::sleep(Duration::from_millis(500));
    if process_exists(pid) {
        show_notification("Recording started", "Press your hotkey to stop", 1200);
        return Ok(());
    }

    remove_pidfile();
    show_notification("Recording failed", &format!("Check {}", LOGFILE), 1600);
    Err(anyhow!("ffmpeg exited immediately"))
}

fn show_notification(title: &str, message: &str, timeout_ms: u32) {
    let _ = Command::new("notify-send")
        .args([
            "-u",
            "normal",
            "-t",
            &timeout_ms.to_string(),
            title,
            message,
        ])
        .status();
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
