use gpui::*;

pub const GRID_GAP: f32 = 14.0;
pub const GRID_PADDING: f32 = 22.0;
pub const GRID_CARD_WIDTH: f32 = 220.0;
pub const GRID_CARD_HEIGHT: f32 = 156.0;
pub const GRID_PREVIEW_WIDTH: f32 = 204.0;
pub const GRID_PREVIEW_HEIGHT: f32 = 114.0;
pub const HEADER_HEIGHT: f32 = 42.0;
pub const PREVIEW_MAX_WIDTH: usize = GRID_PREVIEW_WIDTH as usize;
pub const PREVIEW_MAX_HEIGHT: usize = GRID_PREVIEW_HEIGHT as usize;
pub const GRID_RENDER_PADDING_X_TOTAL: f32 = 40.0;
pub const GRID_RENDER_GAP_X: f32 = 12.0;

pub fn preferred_column_count(window_count: usize, max_columns: usize) -> usize {
    let count = window_count.max(1);
    if count == 1 {
        return 1;
    }
    // Respect the user's max_columns preference.
    // If they want a thin rectangle, this bounds the width.
    let max_cols = max_columns.max(2);
    let cols = count.min(max_cols);
    cols
}

pub fn picker_dimensions(window_count: usize, max_columns: usize, monitor_size: Option<(f32, f32)>) -> (f32, f32) {
    let count = window_count.max(1);
    let cols = preferred_column_count(count, max_columns);
    let width = GRID_PADDING * 2.0
        + cols as f32 * GRID_CARD_WIDTH
        + cols.saturating_sub(1) as f32 * GRID_GAP
        + 24.0;
    let height = picker_height_for(count, cols);

    let (max_w, max_h) = monitor_size
        .map(|(w, h)| (w * 0.9, h * 0.9))
        .unwrap_or((1820.0, 980.0));
    (width.clamp(720.0, max_w), height.clamp(320.0, max_h))
}

pub fn picker_height_for(window_count: usize, columns: usize) -> f32 {
    let count = window_count.max(1);
    let cols = columns.max(1);
    let rows = (count + cols - 1) / cols;
    HEADER_HEIGHT
        + GRID_PADDING * 2.0
        + rows as f32 * GRID_CARD_HEIGHT
        + rows.saturating_sub(1) as f32 * GRID_GAP
}

pub fn rendered_column_count(window: &Window, total_items: usize) -> usize {
    if total_items <= 1 {
        return total_items.max(1);
    }

    let bounds = window.window_bounds().get_bounds();
    let width = bounds.size.width.to_f64() as f32;
    let usable = (width - GRID_RENDER_PADDING_X_TOTAL).max(GRID_CARD_WIDTH);
    let cols = ((usable + GRID_RENDER_GAP_X) / (GRID_CARD_WIDTH + GRID_RENDER_GAP_X)).floor();
    (cols as usize).max(1).min(total_items)
}
