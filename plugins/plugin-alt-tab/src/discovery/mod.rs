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

pub(crate) mod platform;

pub fn on_screen_window_ids() -> Vec<u32> {
    platform::on_screen_window_ids()
}

pub fn get_open_windows() -> Vec<WindowInfo> {
    platform::get_open_windows()
}

pub fn get_on_screen_windows() -> Vec<WindowInfo> {
    platform::get_on_screen_windows()
}
