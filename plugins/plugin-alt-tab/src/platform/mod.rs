#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
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

    pub fn cached_preview_path(_window_id: u32) -> Option<String> {
        None
    }

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

    pub fn move_app_window(_title: &str, _x: i32, _y: i32) {}
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use unsupported as imp;

pub fn cached_preview_path(window_id: u32) -> Option<String> {
    imp::cached_preview_path(window_id)
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

pub fn move_app_window(title: &str, x: i32, y: i32) {
    imp::move_app_window(title, x, y)
}
