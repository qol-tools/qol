mod capture;
mod clipboard;
mod display;
mod ffmpeg;
mod recording;
mod selector;
mod system;

pub use capture::{capture_frozen_frame, capture_screenshot, grab_preview_rgba};
pub use clipboard::{copy_image_to_clipboard, copy_path_to_clipboard};
pub use display::{full_screen_bounds, get_monitors};
pub use recording::{
    capture_log_path, recording_format, recording_started, recording_stopped, start_capture,
    stop_capture,
};
pub use selector::{select_region, select_region_in_app};
pub use system::{
    external_services_check, list_audio_sinks, list_audio_sources, platform_supported_check,
    process_alive, required_binaries_check,
};

pub fn directory_is_read_only(_metadata: &std::fs::Metadata) -> bool {
    false
}

pub fn pre_create_selector(_cx: &mut gpui::App) {}

pub(crate) fn reassert_parked(_title: &str, _cx: &mut gpui::App) {}

pub(crate) fn mark_reveal_requested(_title: &str) {}

pub(crate) fn prepare_preview_window(title: &str) -> bool {
    qol_gpui::popup_window::configure_popup_window(title);
    qol_gpui::popup_window::set_override_redirect_by_title(title);
    qol_gpui::popup_window::hide_invisible(title);
    true
}

pub(crate) fn register_pin_transition(
    _title: &str,
) -> Option<futures::channel::oneshot::Receiver<bool>> {
    None
}

pub(crate) fn complete_pin_transition(_title: &str, _succeeded: bool) {}

pub fn pre_create_pins(cx: &mut gpui::App) {
    crate::ui::pinned::pre_create(cx);
}

pub fn pin_cache_enabled() -> bool {
    false
}

pub fn after_pin_open(_title: &str) {}

pub fn run_internal_mode() -> Option<std::process::ExitCode> {
    qol_windowing::platform::windows::ensure_dpi_awareness();
    recording::run_internal_capture_helper()
}

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

pub fn pin_focus(title: &str) {
    qol_gpui::popup_window::focus_window_by_title(title);
}

pub fn pin_release_focus(title: &str) {
    qol_gpui::popup_window::release_focus_by_title(title);
}

pub fn prepare_pin_window(_title: &str, _origin: (f64, f64)) -> bool {
    false
}

pub fn configure_pin_window(_title: String, _origin: (f64, f64), source_preview: Option<String>) {
    if let Some(source_preview) = source_preview {
        qol_gpui::popup_window::hide_invisible(&source_preview);
        qol_gpui::popup_window::restore_composite(&source_preview);
    }
}
