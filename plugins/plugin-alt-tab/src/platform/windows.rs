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

pub fn capture_preview_rgba(
    _window_id: u32,
    _max_w: usize,
    _max_h: usize,
) -> Option<super::RgbaImage> {
    None
}

pub fn capture_previews_batch_rgba(
    _targets: &[(usize, u32)],
    _max_w: usize,
    _max_h: usize,
) -> Vec<(usize, Option<super::RgbaImage>)> {
    Vec::new()
}

pub fn activate_window(_window_id: u32) {}

pub fn move_app_window(_title: &str, _x: i32, _y: i32) -> bool {
    false
}

pub fn picker_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::PopUp
}

pub fn dismiss_picker(window: &mut gpui::Window) {
    window.minimize_window();
}

pub fn is_modifier_held() -> bool {
    false
}

pub fn is_shift_held() -> bool {
    false
}
