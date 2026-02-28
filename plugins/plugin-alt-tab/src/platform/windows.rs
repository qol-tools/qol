use super::WindowInfo;

pub fn get_open_windows() -> Vec<WindowInfo> {
    Vec::new()
}

pub fn get_on_screen_windows() -> Vec<WindowInfo> {
    Vec::new()
}

pub fn capture_previews_cg(
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

pub fn disable_window_shadow() {}

pub fn get_app_icons(_windows: &[WindowInfo]) -> std::collections::HashMap<String, super::RgbaImage> {
    std::collections::HashMap::new()
}

pub fn close_window(_window_id: u32) {}

pub fn quit_app(_window_id: u32) {}

pub fn minimize_window_by_id(_window_id: u32) {}
