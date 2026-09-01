use anyhow::{anyhow, Context, Result};
use gpui::{Pixels, Point};
use qol_gpui::monitor::ActiveMonitor;
use qol_headless::DoctorCheckResult;
use std::path::Path;
use std::sync::mpsc;

use crate::capture::frozen_frame::FrozenFrame;
use crate::capture::space::CaptureKind;
use crate::platform::CaptureSession;
use crate::{Config, Monitor, Rect};

pub fn pre_create_selector(_cx: &mut gpui::App) {}

pub(crate) fn reassert_parked(_title: &str, _cx: &mut gpui::App) {}

pub(crate) fn mark_reveal_requested(_title: &str) {}

pub fn pre_create_pins(cx: &mut gpui::App) {
    crate::ui::pinned::pre_create(cx);
}

pub fn pin_cache_enabled() -> bool {
    false
}

pub fn after_pin_open(_title: &str) {}

pub fn run_internal_mode() -> Option<std::process::ExitCode> {
    None
}

pub fn select_region(
    _kind: CaptureKind,
    _frozen_frame: Option<FrozenFrame>,
) -> Result<Option<Rect>> {
    Err(anyhow!(
        "qol-shot: region selection is not implemented on Windows"
    ))
}

pub fn select_region_in_app(
    _cx: &mut gpui::App,
    _kind: CaptureKind,
    _cursor: Option<(ActiveMonitor, Option<Point<Pixels>>)>,
    _monitors: Vec<ActiveMonitor>,
    _frozen_frame: Option<FrozenFrame>,
) -> Option<mpsc::Receiver<Option<Rect>>> {
    None
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

pub fn recording_started(_session: &CaptureSession, _countdown_completed: bool) {}

pub fn recording_stopped(
    _session: &CaptureSession,
    _config: &Config,
) -> Option<std::path::PathBuf> {
    None
}

pub fn stop_capture(_session: &CaptureSession) -> Result<()> {
    Err(anyhow!(
        "qol-shot: capture stop is not implemented on Windows"
    ))
}

pub fn process_alive(_pid: u32) -> bool {
    false
}

pub fn show_notification(title: &str, message: &str, _timeout_ms: u32) {
    qol_plugin_daemon::notification::send_notification(title, message);
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

pub fn configure_pin_window(_title: String, _origin: (f64, f64), source_preview: Option<String>) {
    if let Some(source_preview) = source_preview {
        qol_gpui::popup_window::hide_invisible(&source_preview);
        qol_gpui::popup_window::restore_composite(&source_preview);
    }
}

pub fn platform_supported_check() -> DoctorCheckResult {
    DoctorCheckResult::fail(
        "platform_supported",
        "QoL Shot has no Windows capture backend: region selection, screen recording, and clipboard image capture are unsupported.",
    )
    .with_fix("Run QoL Shot on Linux or macOS until a Windows recorder is added.")
}

pub fn required_binaries_check() -> DoctorCheckResult {
    DoctorCheckResult::warn(
        "required_binaries",
        "Windows capture is not implemented, so recorder binaries were not checked.",
    )
}

pub fn permissions_check() -> DoctorCheckResult {
    DoctorCheckResult::fail(
        "permissions",
        "QoL Shot has no supported Windows capture backend.",
    )
    .with_fix("Run QoL Shot on Linux or macOS.")
}

pub fn external_services_check() -> DoctorCheckResult {
    DoctorCheckResult::fail(
        "external_services",
        "QoL Shot cannot inspect display services on unsupported Windows.",
    )
    .with_fix("Run QoL Shot on Linux or macOS.")
}

pub fn list_audio_sources() -> Vec<crate::platform::AudioDevice> {
    Vec::new()
}

pub fn list_audio_sinks() -> Vec<crate::platform::AudioDevice> {
    Vec::new()
}
