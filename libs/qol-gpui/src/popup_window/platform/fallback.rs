pub fn reposition_window_by_title(_title: &str, _gpui_x: f64, _gpui_y: f64) -> bool {
    false
}

pub fn hide_window_by_title(_title: &str) -> bool {
    false
}

pub fn show_window_by_title(_title: &str) -> bool {
    false
}

pub fn disable_window_shadow(_title: &str) -> bool {
    false
}

pub fn configure_popup_window(_title: &str) -> bool {
    false
}

pub fn set_ghost_debug(_opacity: Option<f32>, _color_hex: Option<&str>) {}

pub fn window_backing_scale(_title: &str) -> Option<f32> {
    None
}

pub fn dump_ghost_windows(_context: &str) {}

pub fn prime_hidden_ghost(_window: &gpui::Window, _title: &str) {}
