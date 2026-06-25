use anyhow::{anyhow, Result};
use qol_headless::DoctorCheckResult;
use std::path::Path;

use crate::platform::CaptureSession;
use crate::{Config, Monitor, Rect};

pub fn select_region() -> Result<Option<Rect>> {
    Err(anyhow!(
        "qol-shot: region selection is not implemented on Windows"
    ))
}

pub fn get_monitors() -> Result<Vec<Monitor>> {
    Err(anyhow!(
        "qol-shot: monitor enumeration is not implemented on Windows"
    ))
}

pub fn full_screen_bounds() -> Result<Monitor> {
    Err(anyhow!(
        "qol-shot: full screen bounds are not implemented on Windows"
    ))
}

pub fn start_capture(
    _rect: &Rect,
    _config: &Config,
    _output_file: &Path,
) -> Result<CaptureSession> {
    Err(anyhow!(
        "qol-shot: capture start is not implemented on Windows"
    ))
}

pub fn capture_screenshot(_rect: &Rect, _output_file: &Path) -> Result<()> {
    Err(anyhow!(
        "qol-shot: screenshot capture is not implemented on Windows"
    ))
}

pub fn copy_image_to_clipboard(_path: &Path) -> Result<()> {
    Err(anyhow!(
        "qol-shot: image clipboard copy is not implemented on Windows"
    ))
}

pub fn copy_path_to_clipboard(_path: &Path) -> Result<()> {
    Err(anyhow!(
        "qol-shot: path clipboard copy is not implemented on Windows"
    ))
}

pub fn recording_format(format: &str) -> String {
    format.to_string()
}

pub fn recording_started(_session: &CaptureSession) {
    show_notification("Recording started", "Press your hotkey to stop", 1200);
}

pub fn recording_stopped(_session: &CaptureSession, _config: &Config) {
    show_notification("Recording stopped", "Saved to ~/Videos", 2000);
}

pub fn stop_capture(_session: &CaptureSession) -> Result<()> {
    Err(anyhow!(
        "qol-shot: capture stop is not implemented on Windows"
    ))
}

pub fn process_alive(_pid: u32) -> bool {
    false
}

pub fn show_notification(_title: &str, _message: &str, _timeout_ms: u32) {
    // Notifications are fire-and-forget UX; silently no-op on Windows.
}

pub fn open_url(_url: &str) -> Result<()> {
    Err(anyhow!(
        "qol-shot: URL launcher is not implemented on Windows"
    ))
}

pub fn grab_preview_rgba(_rect: &Rect) -> Option<(Vec<u8>, u32, u32)> {
    None
}

pub fn configure_preview_window(_title: String) {}

pub fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::fail(
        "platform_supported",
        "Windows capture is not implemented for qol-shot.",
    )
    .with_fix("Use the Linux or macOS backend until a Windows recorder is added.")
}

pub fn required_binaries_check() -> DoctorCheckResult {
    DoctorCheckResult::warn(
        "required_binaries",
        "Windows capture is not implemented, so recorder binaries were not checked.",
    )
}
