#[cfg(target_os = "macos")]
pub(crate) mod cg_helpers;

pub use qol_plugin_api::app_icon::RgbaImage;

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app_name: String,
    pub preview_path: Option<String>,
    pub icon: Option<RgbaImage>,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub is_minimized: bool,
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
compile_error!("plugin-alt-tab: unsupported target OS; add src/platform/<os>.rs and wire it in src/platform/mod.rs");

pub fn get_open_windows() -> Vec<WindowInfo> {
    imp::get_open_windows()
}

pub fn get_on_screen_windows() -> Vec<WindowInfo> {
    imp::get_on_screen_windows()
}

pub fn capture_previews_cg(targets: &[(usize, u32)], max_w: usize, max_h: usize) -> Vec<(usize, Option<RgbaImage>)> {
    imp::capture_previews_cg(targets, max_w, max_h)
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

pub fn get_app_icons(windows: &[WindowInfo]) -> std::collections::HashMap<String, RgbaImage> {
    imp::get_app_icons(windows)
}

pub fn disable_window_shadow() {
    imp::disable_window_shadow()
}

pub fn close_window(window_id: u32) {
    imp::close_window(window_id)
}

pub fn quit_app(window_id: u32) {
    imp::quit_app(window_id)
}

pub fn minimize_window_by_id(window_id: u32) {
    imp::minimize_window_by_id(window_id)
}
