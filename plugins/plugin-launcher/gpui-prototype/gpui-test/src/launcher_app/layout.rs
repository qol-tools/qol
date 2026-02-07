use gpui::{px, size, Window};

pub const MAX_VISIBLE: usize = 8;
pub const HEADER_HEIGHT: f32 = 42.0;
pub const ROW_HEIGHT: f32 = 32.0;
pub const WINDOW_WIDTH: f32 = 500.0;

pub fn window_height_for_rows(visible_rows: usize) -> f32 {
    HEADER_HEIGHT + (visible_rows as f32 * ROW_HEIGHT)
}

pub fn resize_for_visible_rows(window_height: &mut f32, visible_rows: usize, window: &mut Window) {
    let target_height = window_height_for_rows(visible_rows);
    if (*window_height - target_height).abs() > f32::EPSILON {
        window.resize(size(px(WINDOW_WIDTH), px(target_height)));
        *window_height = target_height;
    }
}
