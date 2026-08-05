use super::PopupPresentation;

pub struct Platform;

impl PopupPresentation for Platform {
    fn present_topmost(_title: &str) {}

    fn restore_composite(_title: &str) {}
}

#[derive(Clone)]
pub struct WindowGeometrySession;

impl WindowGeometrySession {
    pub fn set_bounds(&self, _x: i32, _y: i32, _width: u32, _height: u32) {}

    pub fn set_position(&self, _x: i32, _y: i32) {}

    pub fn reposition(&self, _x: i32, _y: i32) -> bool {
        false
    }

    pub fn pointer_root(&self) -> Option<(i32, i32)> {
        None
    }

    pub fn bounds(&self) -> Option<(i32, i32, u32, u32)> {
        None
    }

    pub fn anchor_content(&self, _right: bool, _bottom: bool) {}
}

pub fn window_geometry_session(_title: &str) -> Option<WindowGeometrySession> {
    None
}

pub fn window_position_by_title(_title: &str) -> Option<(i32, i32)> {
    None
}

pub fn make_override_redirect(_title: &str) -> bool {
    false
}

pub fn focus_window_by_title(_title: &str) -> bool {
    false
}

pub fn release_focus_by_title(_title: &str) {}

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

pub fn capture_focus_return() {}

pub fn hide_invisible(title: &str) -> bool {
    hide_window_by_title(title)
}

pub fn park_window_by_title(_title: &str) -> bool {
    false
}

pub fn prepare_window_reveal_by_title(_title: &str) -> bool {
    false
}

pub fn configure_keepalive_window(_title: &str) -> bool {
    false
}

pub fn hide_windows_by_title_prefix(_prefix: &str) -> usize {
    0
}

pub fn visible_windows_by_title_prefix(_prefix: &str) -> usize {
    0
}

pub fn hide_for_capture(_title: &str, _window: &mut gpui::Window) -> bool {
    false
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

pub fn show_window_passive_by_title(_title: &str) -> bool {
    false
}

pub fn show_normal_window_by_title(_title: &str) -> bool {
    false
}

pub fn set_window_fixed_size_by_title(_title: &str, _size: gpui::Size<gpui::Pixels>) -> bool {
    false
}

pub fn disable_window_shadow(_title: &str) -> bool {
    false
}

pub fn configure_popup_window(_title: &str) -> bool {
    false
}

pub fn set_window_type_dock_by_title(_title: &str) -> bool {
    false
}

pub fn configure_overlay_window(_title: &str) -> bool {
    false
}

pub fn configure_pinned_window(_title: &str) -> bool {
    false
}

pub fn pinned_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::Normal
}

pub fn set_ghost_debug(_opacity: Option<f32>, _color_hex: Option<&str>) {}

pub fn set_unmap_hide(_enabled: bool) {}

pub fn window_backing_scale(_title: &str) -> Option<f32> {
    None
}

pub fn dump_ghost_windows(_context: &str) {}

pub fn window_holds_input_focus(_title: &str) -> Option<bool> {
    None
}
