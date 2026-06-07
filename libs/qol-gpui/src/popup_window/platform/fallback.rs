pub fn sync_window_layout(
    title: &str,
    window: &mut gpui::Window,
    _origin: gpui::Point<gpui::Pixels>,
    size: gpui::Size<gpui::Pixels>,
) -> bool {
    let backing = window_backing_scale(title);
    crate::window::resize_or_sync_scale(window, size, backing);
    true
}

pub fn hide_invisible(title: &str) -> bool {
    hide_window_by_title(title)
}

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
