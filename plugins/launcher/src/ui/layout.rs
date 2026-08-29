pub const MAX_VISIBLE: usize = 8;
pub const HEADER_HEIGHT: f32 = 42.0;
pub const ROW_HEIGHT: f32 = 32.0;
pub const FLOW_ROW_HEIGHT: f32 = 44.0;
pub const ROW_GAP: f32 = 2.0;
pub const LIST_PAD_Y: f32 = 6.0;
pub const WINDOW_WIDTH: f32 = 500.0;
pub const DETAIL_HEIGHT: f32 = 380.0;

pub fn window_height_for(visible_rows: usize, row_height: f32) -> f32 {
    if visible_rows == 0 {
        return HEADER_HEIGHT + qol_gpui::theme::HEIGHT_HINT_BAR;
    }
    HEADER_HEIGHT
        + 2.0 * LIST_PAD_Y
        + visible_rows as f32 * row_height
        + (visible_rows as f32 - 1.0) * ROW_GAP
        + qol_gpui::theme::HEIGHT_HINT_BAR
}

pub fn window_height_for_rows(visible_rows: usize) -> f32 {
    window_height_for(visible_rows, ROW_HEIGHT)
}

pub fn full_window_height() -> f32 {
    window_height_for_rows(MAX_VISIBLE)
}

pub fn window_height_for_trail() -> f32 {
    HEADER_HEIGHT + qol_gpui::trail::motion::viewport_height() + qol_gpui::theme::HEIGHT_HINT_BAR
}

pub fn window_height_for_detail() -> f32 {
    HEADER_HEIGHT + DETAIL_HEIGHT + qol_gpui::theme::HEIGHT_HINT_BAR
}
