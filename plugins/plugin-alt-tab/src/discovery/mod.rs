pub use qol_plugin_api::app_icon::RgbaImage;

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app_name: String,
    pub preview_path: Option<String>,
    #[allow(dead_code)] // read on Linux, not on macOS
    pub icon: Option<RgbaImage>,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub is_minimized: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // constructed on Linux only (watcher module)
pub(crate) enum CacheEvent {
    WindowsChanged,
}

pub(crate) mod platform;
#[cfg(target_os = "linux")]
pub(crate) mod watcher;

pub fn get_open_windows() -> Vec<WindowInfo> {
    platform::get_open_windows()
}

pub fn get_on_screen_windows() -> Vec<WindowInfo> {
    platform::get_on_screen_windows()
}
