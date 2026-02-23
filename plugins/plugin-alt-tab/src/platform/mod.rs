mod preview;

#[cfg(target_os = "macos")]
pub(crate) mod cg_helpers;

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app_name: String,
    pub preview_path: Option<String>,
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported {
    use super::WindowInfo;

    pub fn get_open_windows() -> Vec<WindowInfo> {
        Vec::new()
    }

    pub fn capture_preview(_window_id: u32, _max_w: usize, _max_h: usize) -> Option<String> {
        None
    }

    pub fn capture_previews_batch(
        _targets: &[(usize, u32)],
        _max_w: usize,
        _max_h: usize,
    ) -> Vec<(usize, Option<String>)> {
        Vec::new()
    }

    pub fn activate_window(_window_id: u32) {}

    pub fn move_app_window(_title: &str, _x: i32, _y: i32) -> bool {
        false
    }

    pub fn is_modifier_held() -> bool {
        false
    }

    pub fn is_shift_held() -> bool {
        false
    }

    pub fn picker_window_kind() -> gpui::WindowKind {
        gpui::WindowKind::PopUp
    }

    pub fn dismiss_picker(window: &mut gpui::Window) {
        window.minimize_window();
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use unsupported as imp;

pub fn cached_preview_path(window_id: u32) -> Option<String> {
    preview::cached_preview_path(window_id)
}

pub fn get_open_windows() -> Vec<WindowInfo> {
    imp::get_open_windows()
}

pub fn capture_preview(window_id: u32, max_w: usize, max_h: usize) -> Option<String> {
    imp::capture_preview(window_id, max_w, max_h)
}

pub fn capture_previews_batch(
    targets: &[(usize, u32)],
    max_w: usize,
    max_h: usize,
) -> Vec<(usize, Option<String>)> {
    imp::capture_previews_batch(targets, max_w, max_h)
}

pub fn activate_window(window_id: u32) {
    imp::activate_window(window_id)
}

pub fn move_app_window(title: &str, x: i32, y: i32) -> bool {
    imp::move_app_window(title, x, y)
}

pub fn is_modifier_held() -> bool {
    imp::is_modifier_held()
}

pub fn is_shift_held() -> bool {
    imp::is_shift_held()
}

pub fn picker_window_kind() -> gpui::WindowKind {
    imp::picker_window_kind()
}

pub fn dismiss_picker(window: &mut gpui::Window) {
    imp::dismiss_picker(window)
}