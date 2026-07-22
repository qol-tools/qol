use anyhow::{Context, Result};
use qol_headless::DoctorCheckResult;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::capture::frozen_frame::FrozenFrame;
use crate::Rect;

pub fn process_alive(pid: u32) -> bool {
    crate::platform::unix_process_alive(pid)
}

pub fn show_notification(title: &str, message: &str, _timeout_ms: u32) {
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        escape_applescript(message),
        escape_applescript(title)
    );

    let _ = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

pub fn show_saved_notification(
    title: &str,
    message: &str,
    timeout_ms: u32,
    _target: crate::capture::completion::RevealTarget,
) {
    show_notification(title, message, timeout_ms);
}

pub fn open_url(url: &str) -> Result<()> {
    qol_apps::desktop_integration::open_with_default_app(url).context("failed to open URL")
}

pub fn grab_preview_rgba(_rect: &Rect) -> Option<(Vec<u8>, u32, u32)> {
    None
}

pub fn capture_frozen_frame() -> Result<Option<FrozenFrame>> {
    Ok(None)
}

pub fn configure_preview_window(_title: String) {}

#[derive(Clone)]
pub struct PinResizeSession;

pub fn pin_resize_session(_title: &str) -> Option<PinResizeSession> {
    None
}

impl PinResizeSession {
    pub fn apply(&self, _x: f32, _y: f32, _width: f32, _height: f32) {}

    pub fn move_to(&self, _x: f32, _y: f32) {}

    pub fn pointer(&self) -> Option<(f32, f32)> {
        None
    }

    pub fn bounds(&self) -> Option<(f32, f32, f32, f32)> {
        None
    }

    pub fn anchor(&self, _right: bool, _bottom: bool) {}
}

pub fn pin_focus(_title: &str) {}

pub fn pin_release_focus(_title: &str) {}

pub fn prepare_pin_window(_title: &str, _origin: (f64, f64)) -> bool {
    false
}

pub fn configure_pin_window(title: String, _origin: (f64, f64), source_preview: Option<String>) {
    qol_gpui::popup_window::configure_pinned_window(&title);
    if let Some(source_preview) = source_preview {
        qol_gpui::popup_window::hide_invisible(&source_preview);
        qol_gpui::popup_window::restore_composite(&source_preview);
    }
}

pub fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::ok(
        "platform_supported",
        "macOS capture is supported through screencapture.",
    )
}

pub fn required_binaries_check() -> DoctorCheckResult {
    let required = ["screencapture", "open", "osascript"];
    let missing = required
        .iter()
        .copied()
        .filter(|name| resolve_command(name).is_none())
        .collect::<Vec<_>>();

    if !missing.is_empty() {
        return DoctorCheckResult::fail(
            "required_binaries",
            format!("Missing required binaries: {}.", missing.join(", ")),
        )
        .with_fix("Restore the missing macOS command line tools.");
    }

    if resolve_command("ffmpeg").is_none() && resolve_command("avconvert").is_none() {
        return DoctorCheckResult::warn(
            "required_binaries",
            "Native MOV recording is available, including multi-display composition, but non-MOV conversion is limited without ffmpeg or avconvert.",
        )
        .with_fix("Install ffmpeg for MP4, MKV, or WebM output, or avconvert for MP4 output.");
    }

    if resolve_command("ffmpeg").is_none() {
        return DoctorCheckResult::warn(
            "required_binaries",
            "Native MOV recording and MP4 conversion through avconvert are available, including multi-display composition, but WebM/MKV require ffmpeg.",
        )
        .with_fix("Install ffmpeg for WebM or MKV output.");
    }

    DoctorCheckResult::ok(
        "required_binaries",
        "Required macOS capture, composition, and conversion tools are available.",
    )
}

pub(super) fn signal_process(pid: u32, signal: i32) -> Result<()> {
    crate::platform::unix_signal_process(pid, signal)
}

pub(super) fn wait_for_process_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !process_alive(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(150));
    }
    !process_alive(pid)
}

pub(super) fn open_path(path: &Path) -> bool {
    qol_apps::desktop_integration::open_with_default_app(path).is_ok()
}

pub(super) fn videos_dir() -> Option<PathBuf> {
    let home = env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Videos"))
}

pub(super) fn capture_work_dir() -> PathBuf {
    env::temp_dir().join("qol-shot")
}

pub(super) fn ensure_capture_work_dir() -> Result<PathBuf> {
    let dir = capture_work_dir();
    fs::create_dir_all(&dir).context("failed to create capture work directory")?;
    Ok(dir)
}

pub(super) fn move_file(source: &Path, destination: &Path) -> Result<()> {
    if fs::rename(source, destination).is_ok() {
        return Ok(());
    }

    fs::copy(source, destination).with_context(|| {
        format!(
            "failed to move {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    let _ = fs::remove_file(source);
    Ok(())
}

pub(super) fn paths_match(left: &Path, right: &Path) -> bool {
    left == right
}

pub(super) fn path_extension_is(path: &Path, expected: &str) -> bool {
    output_extension(path).as_deref() == Some(expected)
}

pub(super) fn output_extension(path: &Path) -> Option<String> {
    path.extension()?
        .to_str()
        .map(|ext| ext.to_ascii_lowercase())
}

pub(super) fn output_format_label(path: &Path) -> String {
    output_extension(path)
        .map(|ext| ext.to_ascii_uppercase())
        .unwrap_or_else(|| "recording".to_string())
}

fn escape_applescript(input: &str) -> String {
    input.replace('\\', "\\\\").replace('"', "\\\"")
}

pub(super) fn resolve_command(command: &str) -> Option<PathBuf> {
    let mut dirs = env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .unwrap_or_default();
    dirs.extend([
        PathBuf::from("/usr/bin"),
        PathBuf::from("/usr/sbin"),
        PathBuf::from("/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    dirs.into_iter()
        .map(|dir| dir.join(command))
        .find(|path| path.is_file())
}

pub fn list_audio_sources() -> Vec<crate::platform::AudioDevice> {
    Vec::new()
}

pub fn list_audio_sinks() -> Vec<crate::platform::AudioDevice> {
    Vec::new()
}
