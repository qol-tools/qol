mod clipboard;
mod conversion;
mod display;
mod labels;
mod overlay;
mod recording;
mod selector;
mod swift;
mod system;

pub use clipboard::{copy_image_to_clipboard, copy_path_to_clipboard};
pub use display::{full_screen_bounds, get_monitors};
pub use recording::{
    capture_screenshot, recording_format, recording_started, recording_stopped, start_capture,
    stop_capture,
};
pub use selector::{select_region, select_region_in_app};
pub use system::{
    configure_pin_window, configure_preview_window, grab_preview_rgba, open_url,
    pin_resize_session, platform_supported_check, process_alive, required_binaries_check,
    show_notification, PinResizeSession,
};

#[cfg(test)]
mod tests;
