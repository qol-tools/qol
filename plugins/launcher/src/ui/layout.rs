pub const MAX_VISIBLE: usize = 8;
pub const HEADER_HEIGHT: f32 = 42.0;
pub const ROW_HEIGHT: f32 = 32.0;
pub const ROW_GAP: f32 = 2.0;
pub const LIST_PAD_Y: f32 = 6.0;
pub const WINDOW_WIDTH: f32 = 500.0;

pub fn window_height_for_rows(visible_rows: usize) -> f32 {
    if visible_rows == 0 {
        return HEADER_HEIGHT + qol_gpui::theme::HEIGHT_HINT_BAR;
    }
    HEADER_HEIGHT
        + 2.0 * LIST_PAD_Y
        + visible_rows as f32 * ROW_HEIGHT
        + (visible_rows as f32 - 1.0) * ROW_GAP
        + qol_gpui::theme::HEIGHT_HINT_BAR
}

pub fn full_window_height() -> f32 {
    window_height_for_rows(MAX_VISIBLE)
}
