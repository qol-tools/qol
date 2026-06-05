pub fn is_modifier_held() -> bool {
    false
}

pub fn is_shift_held() -> bool {
    false
}

pub fn set_accessory_policy() {}

pub fn ghost_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::PopUp
}

pub fn ghost_window_decorations(_transparent: bool) -> gpui::WindowDecorations {
    gpui::WindowDecorations::Client
}

pub fn adjust_ghost_bounds(bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
    bounds
}

pub fn should_poll_focus() -> bool {
    false
}

pub fn has_process_focus() -> bool {
    true
}
