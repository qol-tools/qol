pub const MAX_VISIBLE: usize = 8;
pub const HEADER_HEIGHT: f32 = 42.0;
pub const ROW_HEIGHT: f32 = 32.0;
pub const WINDOW_WIDTH: f32 = 500.0;

pub fn window_height_for_rows(visible_rows: usize) -> f32 {
    HEADER_HEIGHT + (visible_rows as f32 * ROW_HEIGHT)
}

pub fn full_window_height() -> f32 {
    window_height_for_rows(MAX_VISIBLE)
}
