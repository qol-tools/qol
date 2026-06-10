pub const MIN_CARD_SCALE: f32 = 0.5;
pub const MAX_CARD_SCALE: f32 = 2.5;
pub const DEFAULT_CARD_SCALE: f32 = 1.5;

const BASE_CARD_WIDTH: f32 = 220.0;
const BASE_CARD_HEIGHT: f32 = 156.0;
const BASE_PREVIEW_WIDTH: f32 = 204.0;
const BASE_PREVIEW_HEIGHT: f32 = 114.0;
const BASE_LABEL_FONT: f32 = 12.0;
const BASE_LABEL_ICON: f32 = 16.0;
const BASE_MINIMIZED_ICON: f32 = 48.0;
/// Height of the hotkey hints bar (py_2 + text_xs + border_b_1).
pub const HOTKEY_HINTS_HEIGHT: f32 = 48.0;
/// Capture ceiling: previews are captured once at the largest size any
/// card scale can display, then downscaled by the renderer.
pub const PREVIEW_MAX_WIDTH: usize = (BASE_PREVIEW_WIDTH * MAX_CARD_SCALE) as usize;
pub const PREVIEW_MAX_HEIGHT: usize = (BASE_PREVIEW_HEIGHT * MAX_CARD_SCALE) as usize;
/// Matches render: px_5 * 2 = 40px horizontal, py_4 * 2 = 32px vertical, gap_3 = 12px.
const RENDER_PAD_X: f32 = 40.0;
const RENDER_PAD_Y: f32 = 32.0;
const RENDER_GAP: f32 = 12.0;
/// Slack added to the panel width so float rounding in the flex-wrap pass
/// can never wrap the last card of a row early.
const WIDTH_SLACK: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CardMetrics {
    pub scale: f32,
    pub card_width: f32,
    pub card_height: f32,
    pub preview_width: f32,
    pub preview_height: f32,
}

impl CardMetrics {
    pub fn from_scale(scale: f32) -> Self {
        let s = clamp_card_scale(scale);
        Self {
            scale: s,
            card_width: BASE_CARD_WIDTH * s,
            card_height: BASE_CARD_HEIGHT * s,
            preview_width: BASE_PREVIEW_WIDTH * s,
            preview_height: BASE_PREVIEW_HEIGHT * s,
        }
    }

    pub fn label_font_px(&self, size_factor: f32) -> f32 {
        BASE_LABEL_FONT * self.scale * size_factor
    }

    pub fn label_icon_px(&self, size_factor: f32) -> f32 {
        BASE_LABEL_ICON * self.scale * size_factor
    }

    pub fn minimized_icon_px(&self) -> f32 {
        BASE_MINIMIZED_ICON * self.scale
    }
}

pub fn clamp_card_scale(scale: f32) -> f32 {
    if scale.is_finite() {
        scale.clamp(MIN_CARD_SCALE, MAX_CARD_SCALE)
    } else {
        DEFAULT_CARD_SCALE
    }
}

/// The single source of truth for picker geometry. Render sizes the panel
/// from this and arrow navigation moves by `columns` from this, so the two
/// can never disagree on the grid shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickerLayout {
    pub width: f32,
    pub height: f32,
    pub columns: usize,
}

pub fn picker_layout(
    window_count: usize,
    max_columns: usize,
    monitor_size: Option<(f32, f32)>,
    show_hotkey_hints: bool,
    card_scale: f32,
) -> PickerLayout {
    let metrics = CardMetrics::from_scale(card_scale);
    let count = window_count.max(1);
    let (max_w, max_h) = monitor_size
        .map(|(w, h)| (w * 0.9, h * 0.9))
        .unwrap_or((1820.0, 980.0));

    let preferred = preferred_column_count(count, max_columns);
    let columns = if width_for_cols(preferred, &metrics) <= max_w {
        preferred
    } else {
        cols_for_width(max_w, count, &metrics)
    };
    let width = (width_for_cols(columns, &metrics) + WIDTH_SLACK).min(max_w);

    let hints_height = if show_hotkey_hints {
        HOTKEY_HINTS_HEIGHT
    } else {
        0.0
    };
    let height = (picker_height_for(count, columns, &metrics) + hints_height).min(max_h);
    PickerLayout {
        width,
        height,
        columns,
    }
}

pub fn preferred_column_count(window_count: usize, max_columns: usize) -> usize {
    let count = window_count.max(1);
    if count == 1 {
        return 1;
    }
    let max_cols = max_columns.max(2);
    count.min(max_cols)
}

fn width_for_cols(cols: usize, metrics: &CardMetrics) -> f32 {
    RENDER_PAD_X + cols as f32 * metrics.card_width + cols.saturating_sub(1) as f32 * RENDER_GAP
}

fn cols_for_width(width: f32, max_items: usize, metrics: &CardMetrics) -> usize {
    let usable = (width - RENDER_PAD_X).max(metrics.card_width);
    let cols = ((usable + RENDER_GAP) / (metrics.card_width + RENDER_GAP)).floor();
    (cols as usize).max(1).min(max_items)
}

fn picker_height_for(window_count: usize, columns: usize, metrics: &CardMetrics) -> f32 {
    let count = window_count.max(1);
    let cols = columns.max(1);
    let rows = count.div_ceil(cols);
    RENDER_PAD_Y + rows as f32 * metrics.card_height + rows.saturating_sub(1) as f32 * RENDER_GAP
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_metrics_scale_and_clamp() {
        let cases = [
            (1.0, (220.0, 156.0, 204.0, 114.0)),
            (1.5, (330.0, 234.0, 306.0, 171.0)),
            (0.1, (110.0, 78.0, 102.0, 57.0)),
            (10.0, (550.0, 390.0, 510.0, 285.0)),
            (f32::NAN, (330.0, 234.0, 306.0, 171.0)),
        ];
        for (scale, (cw, ch, pw, ph)) in cases {
            let m = CardMetrics::from_scale(scale);
            assert_eq!(
                (
                    m.card_width,
                    m.card_height,
                    m.preview_width,
                    m.preview_height
                ),
                (cw, ch, pw, ph),
                "scale: {scale}"
            );
        }
    }

    #[test]
    fn columns_match_what_the_width_can_render() {
        let budgets = [
            None,
            Some((1280.0, 800.0)),
            Some((1920.0, 1080.0)),
            Some((3440.0, 1440.0)),
        ];
        let scales = [0.5, 1.0, 1.5, 2.0, 2.5];
        let counts = [1, 2, 3, 5, 6, 7, 12, 30];
        for budget in budgets {
            for scale in scales {
                for count in counts {
                    let layout = picker_layout(count, 6, budget, true, scale);
                    let metrics = CardMetrics::from_scale(scale);
                    let wrapped = cols_for_width(layout.width, count, &metrics);
                    assert_eq!(
                        layout.columns, wrapped,
                        "budget={budget:?} scale={scale} count={count}: \
                         nav columns {} but width {} wraps to {}",
                        layout.columns, layout.width, wrapped
                    );
                }
            }
        }
    }

    #[test]
    fn width_hugs_columns_exactly() {
        let cases = [(1, 1.0), (1, 2.5), (4, 1.5), (6, 1.5), (12, 2.5), (30, 0.5)];
        for (count, scale) in cases {
            let layout = picker_layout(count, 6, Some((3440.0, 1440.0)), false, scale);
            let metrics = CardMetrics::from_scale(scale);
            let exact = width_for_cols(layout.columns, &metrics);
            assert!(
                layout.width >= exact && layout.width <= exact + WIDTH_SLACK,
                "count={count} scale={scale}: width {} not hugging content {}",
                layout.width,
                exact
            );
        }
    }

    #[test]
    fn layout_stays_within_monitor() {
        let cases = [(1, 1.0), (4, 1.5), (12, 2.5), (30, 2.5), (30, 0.5)];
        for (count, scale) in cases {
            let layout = picker_layout(count, 6, Some((1280.0, 800.0)), true, scale);
            assert!(
                layout.width <= 1280.0 * 0.9 && layout.height <= 800.0 * 0.9,
                "count={count} scale={scale}: {}x{} exceeds monitor budget",
                layout.width,
                layout.height
            );
            assert!(layout.columns >= 1, "count={count} scale={scale}");
        }
    }

    #[test]
    fn single_window_gets_single_column_panel() {
        let layout = picker_layout(1, 6, Some((1920.0, 1080.0)), false, 1.5);
        let metrics = CardMetrics::from_scale(1.5);
        assert_eq!(layout.columns, 1);
        assert!(layout.width <= width_for_cols(1, &metrics) + WIDTH_SLACK);
    }

    #[test]
    fn capture_ceiling_covers_max_scale() {
        let m = CardMetrics::from_scale(MAX_CARD_SCALE);
        assert!(PREVIEW_MAX_WIDTH as f32 >= m.preview_width);
        assert!(PREVIEW_MAX_HEIGHT as f32 >= m.preview_height);
    }
}
