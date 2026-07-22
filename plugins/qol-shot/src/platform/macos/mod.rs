mod clipboard;
mod conversion;
mod display;
mod labels;
mod overlay;
mod recording;
mod selector;
mod status;
mod swift;
mod system;

pub use clipboard::{copy_image_to_clipboard, copy_path_to_clipboard};
pub use display::{full_screen_bounds, get_monitors};
pub use recording::{
    capture_screenshot, recording_format, recording_started, recording_stopped, start_capture,
    stop_capture,
};
pub use selector::{select_region, select_region_in_app};

pub fn pre_create_selector(_cx: &mut gpui::App) {}

pub fn pre_create_pins(cx: &mut gpui::App) {
    crate::ui::pinned::pre_create(cx);
}

pub fn pin_cache_enabled() -> bool {
    false
}

pub fn after_pin_open(_title: &str) {}
pub use status::{hide_capture_status, show_capture_status};
pub use system::{
    capture_frozen_frame, configure_pin_window, configure_preview_window, grab_preview_rgba,
    list_audio_sinks, list_audio_sources, open_url, pin_focus, pin_release_focus,
    pin_resize_session, platform_supported_check, prepare_pin_window, process_alive,
    required_binaries_check, show_notification, show_saved_notification, PinResizeSession,
};

#[cfg(test)]
mod tests;
