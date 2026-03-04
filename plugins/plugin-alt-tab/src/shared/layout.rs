use gpui::*;

pub const GRID_CARD_WIDTH: f32 = 220.0;
pub const GRID_CARD_HEIGHT: f32 = 156.0;
pub const GRID_PREVIEW_WIDTH: f32 = 204.0;
pub const GRID_PREVIEW_HEIGHT: f32 = 114.0;
/// Height of the hotkey hints bar (py_2 + text_xs + border_b_1).
pub const HOTKEY_HINTS_HEIGHT: f32 = 48.0;
pub const PREVIEW_MAX_WIDTH: usize = GRID_PREVIEW_WIDTH as usize;
pub const PREVIEW_MAX_HEIGHT: usize = GRID_PREVIEW_HEIGHT as usize;
/// Matches render: px_5 * 2 = 40px horizontal, py_4 * 2 = 32px vertical, gap_3 = 12px.
const RENDER_PAD_X: f32 = 40.0;
const RENDER_PAD_Y: f32 = 32.0;
const RENDER_GAP: f32 = 12.0;

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

pub fn picker_dimensions(window_count: usize, max_columns: usize, monitor_size: Option<(f32, f32)>, show_hotkey_hints: bool) -> (f32, f32) {
    let count = window_count.max(1);
    let cols = preferred_column_count(count, max_columns);
    let width = width_for_cols(cols);

    let (max_w, max_h) = monitor_size
        .map(|(w, h)| (w * 0.9, h * 0.9))
        .unwrap_or((1820.0, 980.0));
    let clamped_w = width.clamp(720.0, max_w);

    // Recalculate columns that actually fit after clamping — prevents
    // height being calculated for more columns than the window can show.
    let actual_cols = cols_for_width(clamped_w, count);

    let hints_height = if show_hotkey_hints { HOTKEY_HINTS_HEIGHT } else { 0.0 };
    let height = picker_height_for(count, actual_cols) + hints_height;
    (clamped_w, height.clamp(320.0, max_h))
}

fn width_for_cols(cols: usize) -> f32 {
    RENDER_PAD_X + cols as f32 * GRID_CARD_WIDTH + cols.saturating_sub(1) as f32 * RENDER_GAP
}

fn cols_for_width(width: f32, max_items: usize) -> usize {
    let usable = (width - RENDER_PAD_X).max(GRID_CARD_WIDTH);
    let cols = ((usable + RENDER_GAP) / (GRID_CARD_WIDTH + RENDER_GAP)).floor();
    (cols as usize).max(1).min(max_items)
}

pub fn picker_height_for(window_count: usize, columns: usize) -> f32 {
    let count = window_count.max(1);
    let cols = columns.max(1);
    let rows = (count + cols - 1) / cols;
    RENDER_PAD_Y + rows as f32 * GRID_CARD_HEIGHT + rows.saturating_sub(1) as f32 * RENDER_GAP
}

pub fn rendered_column_count(window: &Window, total_items: usize) -> usize {
    if total_items <= 1 {
        return total_items.max(1);
    }
    let bounds = window.window_bounds().get_bounds();
    let width = bounds.size.width.to_f64() as f32;
    cols_for_width(width, total_items)
}
