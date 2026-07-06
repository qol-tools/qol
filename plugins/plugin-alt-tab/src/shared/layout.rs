pub const MIN_CARD_SCALE: f32 = 0.5;
pub const MAX_CARD_SCALE: f32 = 2.5;
pub const DEFAULT_CARD_SCALE: f32 = 1.5;
pub const MIN_CARD_PADDING: f32 = 0.0;
pub const MAX_CARD_PADDING: f32 = 24.0;
pub const DEFAULT_CARD_PADDING: f32 = 4.0;

const BASE_CARD_WIDTH: f32 = 220.0;
const PREVIEW_ASPECT_W: f32 = 16.0;
const PREVIEW_ASPECT_H: f32 = 9.0;
const BASE_LABEL_FONT: f32 = 10.0;
const BASE_LABEL_STRIP_HEIGHT: f32 = BASE_LABEL_FONT * 1.75;
const LABEL_LINE_HEIGHT_FACTOR: f32 = 1.25;
const BASE_LABEL_ICON: f32 = 16.0;
const BASE_MINIMIZED_ICON: f32 = 48.0;
/// Height of the hotkey hints bar (py_2 + text_xs + border_b_1).
pub const HOTKEY_HINTS_HEIGHT: f32 = 48.0;
/// Capture ceiling: previews are captured once at the largest size any
/// card scale can display, then downscaled by the renderer.
pub const PREVIEW_MAX_WIDTH: usize = (BASE_CARD_WIDTH * MAX_CARD_SCALE + 1.0) as usize;
pub const PREVIEW_MAX_HEIGHT: usize =
    (BASE_CARD_WIDTH * MAX_CARD_SCALE * PREVIEW_ASPECT_H / PREVIEW_ASPECT_W + 1.0) as usize;
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
    pub card_padding: f32,
    pub card_width: f32,
    pub card_height: f32,
    pub preview_width: f32,
    pub preview_height: f32,
    pub label_strip_height: f32,
}

impl CardMetrics {
    pub fn from_config(scale: f32, card_padding: f32) -> Self {
        let s = clamp_card_scale(scale);
        let card_padding = clamp_card_padding(card_padding);
        let card_width = BASE_CARD_WIDTH * s;
        let preview_width = (card_width - card_padding * 2.0).max(1.0);
        let preview_height = preview_width * PREVIEW_ASPECT_H / PREVIEW_ASPECT_W;
        let label_strip_height = BASE_LABEL_STRIP_HEIGHT * s;
        Self {
            scale: s,
            card_padding,
            card_width,
            card_height: preview_height + label_strip_height + card_padding * 2.0,
            preview_width,
            preview_height,
            label_strip_height,
        }
    }

    pub fn label_font_px(&self, size_factor: f32) -> f32 {
        BASE_LABEL_FONT * self.scale * size_factor
    }

    pub fn label_line_height_px(&self, size_factor: f32) -> f32 {
        self.label_font_px(size_factor) * LABEL_LINE_HEIGHT_FACTOR
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

pub fn clamp_card_padding(padding: f32) -> f32 {
    if padding.is_finite() {
        padding.clamp(MIN_CARD_PADDING, MAX_CARD_PADDING)
    } else {
        DEFAULT_CARD_PADDING
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
    pub metrics: CardMetrics,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreviewRect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

pub fn picker_layout(
    window_count: usize,
    max_columns: usize,
    monitor_size: Option<(f32, f32)>,
    show_hotkey_hints: bool,
    card_scale: f32,
    card_padding: f32,
    dynamic_card_scale: bool,
) -> PickerLayout {
    let count = window_count.max(1);
    let padding = clamp_card_padding(card_padding);
    let (max_w, max_h) = monitor_size
        .map(|(w, h)| (w * 0.9, h * 0.9))
        .unwrap_or((1820.0, 980.0));
    let hints_height = if show_hotkey_hints {
        HOTKEY_HINTS_HEIGHT
    } else {
        0.0
    };
    let grid_max_h = max_h - hints_height;

    let (columns, scale) = if dynamic_card_scale {
        best_dynamic_fit(count, max_columns, max_w, grid_max_h, padding)
    } else {
        fixed_fit(count, max_columns, max_w, grid_max_h, padding, card_scale)
    };

    let metrics = CardMetrics::from_config(scale, padding);
    let width = (width_for_cols(columns, &metrics) + WIDTH_SLACK).min(max_w);
    let height = (picker_height_for(count, columns, &metrics) + hints_height).min(max_h);
    PickerLayout {
        width,
        height,
        columns,
        metrics,
    }
}

fn best_dynamic_fit(
    count: usize,
    max_columns: usize,
    max_w: f32,
    max_h: f32,
    padding: f32,
) -> (usize, f32) {
    let cap = count.min(max_columns.max(2)).max(1);
    let mut best = (1usize, f32::NEG_INFINITY);
    for cols in 1..=cap {
        let rows = count.div_ceil(cols);
        let fit = fit_scale(cols, rows, max_w, max_h, padding).min(MAX_CARD_SCALE);
        if fit >= best.1 {
            best = (cols, fit);
        }
    }
    (best.0, best.1.max(MIN_CARD_SCALE))
}

fn fixed_fit(
    count: usize,
    max_columns: usize,
    max_w: f32,
    max_h: f32,
    padding: f32,
    card_scale: f32,
) -> (usize, f32) {
    let configured = clamp_card_scale(card_scale);
    let cap = count.min(max_columns.max(2)).max(1);
    let mut best = (1usize, f32::NEG_INFINITY);
    for columns in 1..=cap {
        let rows = count.div_ceil(columns);
        let fit = fit_scale(columns, rows, max_w, max_h, padding).min(configured);
        if fit >= best.1 {
            best = (columns, fit);
        }
    }
    (best.0, best.1.max(MIN_CARD_SCALE))
}

fn fit_scale(cols: usize, rows: usize, max_w: f32, max_h: f32, padding: f32) -> f32 {
    let cols_f = cols.max(1) as f32;
    let rows_f = rows.max(1) as f32;
    let aspect = PREVIEW_ASPECT_H / PREVIEW_ASPECT_W;
    let width_budget = max_w - RENDER_PAD_X - WIDTH_SLACK - (cols_f - 1.0) * RENDER_GAP;
    let scale_w = width_budget / (cols_f * BASE_CARD_WIDTH);
    let row_budget = (max_h - RENDER_PAD_Y - (rows_f - 1.0) * RENDER_GAP) / rows_f;
    let height_slope = BASE_CARD_WIDTH * aspect + BASE_LABEL_STRIP_HEIGHT;
    let height_intercept = 2.0 * padding * (1.0 - aspect);
    let scale_h = (row_budget - height_intercept) / height_slope;
    scale_w.min(scale_h)
}

fn width_for_cols(cols: usize, metrics: &CardMetrics) -> f32 {
    RENDER_PAD_X + cols as f32 * metrics.card_width + cols.saturating_sub(1) as f32 * RENDER_GAP
}

#[cfg(test)]
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

pub fn preview_rect_for_card(
    index: usize,
    columns: usize,
    panel_origin: (f32, f32),
    show_hotkey_hints: bool,
    metrics: &CardMetrics,
) -> PreviewRect {
    let columns = columns.max(1);
    let row = index / columns;
    let col = index % columns;
    let header_h = if show_hotkey_hints {
        HOTKEY_HINTS_HEIGHT
    } else {
        0.0
    };
    let card_x =
        panel_origin.0 + RENDER_PAD_X / 2.0 + col as f32 * (metrics.card_width + RENDER_GAP);
    let card_y = panel_origin.1
        + header_h
        + RENDER_PAD_Y / 2.0
        + row as f32 * (metrics.card_height + RENDER_GAP);
    PreviewRect {
        x: card_x + metrics.card_padding,
        y: card_y + metrics.card_padding,
        w: metrics.preview_width,
        h: metrics.preview_height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_metrics_scale_and_clamp() {
        let cases = [
            (1.0, (220.0, 144.75, 212.0, 119.25, 17.5)),
            (1.5, (330.0, 215.375, 322.0, 181.125, 26.25)),
            (0.1, (110.0, 74.125, 102.0, 57.375, 8.75)),
            (10.0, (550.0, 356.625, 542.0, 304.875, 43.75)),
            (f32::NAN, (330.0, 215.375, 322.0, 181.125, 26.25)),
        ];
        for (scale, (cw, ch, pw, ph, lh)) in cases {
            let m = CardMetrics::from_config(scale, DEFAULT_CARD_PADDING);
            assert_close(m.card_width, cw, &format!("scale {scale}: card_width"));
            assert_close(m.card_height, ch, &format!("scale {scale}: card_height"));
            assert_close(
                m.preview_width,
                pw,
                &format!("scale {scale}: preview_width"),
            );
            assert_close(
                m.preview_height,
                ph,
                &format!("scale {scale}: preview_height"),
            );
            assert_close(
                m.label_strip_height,
                lh,
                &format!("scale {scale}: label_strip_height"),
            );
            assert_close(
                m.card_padding,
                DEFAULT_CARD_PADDING,
                &format!("scale {scale}: card_padding"),
            );
        }
    }

    fn assert_close(actual: f32, expected: f32, context: &str) {
        assert!(
            (actual - expected).abs() <= 0.0001,
            "{context}: got {actual}, expected {expected}"
        );
    }

    fn content_height(layout: &PickerLayout, count: usize, hints: bool) -> f32 {
        let rows = count.max(1).div_ceil(layout.columns.max(1));
        let hints_h = if hints { HOTKEY_HINTS_HEIGHT } else { 0.0 };
        RENDER_PAD_Y
            + rows as f32 * layout.metrics.card_height
            + rows.saturating_sub(1) as f32 * RENDER_GAP
            + hints_h
    }

    #[test]
    fn card_padding_is_configurable_and_clamped() {
        let cases = [
            (0.0, (330.0, 185.625, 0.0)),
            (12.0, (306.0, 172.125, 12.0)),
            (100.0, (282.0, 158.625, MAX_CARD_PADDING)),
            (f32::NAN, (322.0, 181.125, DEFAULT_CARD_PADDING)),
        ];
        for (padding, (pw, ph, cp)) in cases {
            let m = CardMetrics::from_config(1.5, padding);
            assert_eq!(
                (m.preview_width, m.preview_height, m.card_padding),
                (pw, ph, cp),
                "padding: {padding}"
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
                    let layout =
                        picker_layout(count, 6, budget, true, scale, DEFAULT_CARD_PADDING, false);
                    let wrapped = cols_for_width(layout.width, count, &layout.metrics);
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
            let layout = picker_layout(
                count,
                6,
                Some((3440.0, 1440.0)),
                false,
                scale,
                DEFAULT_CARD_PADDING,
                false,
            );
            let exact = width_for_cols(layout.columns, &layout.metrics);
            assert!(
                layout.width >= exact && layout.width <= exact + WIDTH_SLACK,
                "count={count} scale={scale}: width {} not hugging content {}",
                layout.width,
                exact
            );
        }
    }

    #[test]
    fn preview_rect_matches_grid_padding_gap_and_card_padding() {
        let metrics = CardMetrics::from_config(1.5, 4.0);
        let rect = preview_rect_for_card(7, 6, (100.0, 200.0), true, &metrics);

        assert_close(rect.x, 100.0 + 20.0 + 1.0 * (330.0 + 12.0) + 4.0, "x");
        assert_close(
            rect.y,
            200.0 + HOTKEY_HINTS_HEIGHT + 16.0 + 1.0 * (215.375 + 12.0) + 4.0,
            "y",
        );
        assert_close(rect.w, metrics.preview_width, "w");
        assert_close(rect.h, metrics.preview_height, "h");
    }

    #[test]
    fn layout_stays_within_monitor() {
        let cases = [(1, 1.0), (4, 1.5), (12, 2.5), (30, 2.5), (30, 0.5)];
        for (count, scale) in cases {
            let layout = picker_layout(
                count,
                6,
                Some((1280.0, 800.0)),
                true,
                scale,
                DEFAULT_CARD_PADDING,
                false,
            );
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
    fn grid_content_always_fits_monitor_budget() {
        let budgets = [(1280.0, 800.0), (1920.0, 1080.0), (3440.0, 1440.0)];
        let scales = [0.5, 1.5, 2.5];
        let counts = [1, 2, 3, 6, 12, 30];
        for dynamic in [false, true] {
            for (bw, bh) in budgets {
                for scale in scales {
                    for count in counts {
                        let layout = picker_layout(
                            count,
                            6,
                            Some((bw, bh)),
                            true,
                            scale,
                            DEFAULT_CARD_PADDING,
                            dynamic,
                        );
                        let fits = content_height(&layout, count, true) <= bh * 0.9 + 0.01
                            && layout.width <= bw * 0.9 + 0.01;
                        let at_floor = (layout.metrics.scale - MIN_CARD_SCALE).abs() < 0.0001;
                        assert!(
                            fits || at_floor,
                            "dynamic={dynamic} budget={bw}x{bh} scale={scale} count={count}: \
                             content overflows without hitting the scale floor"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn dynamic_scale_grows_for_few_windows() {
        let cases = [(1, 1), (2, 2), (3, 3)];
        for (count, expected_columns) in cases {
            let layout = picker_layout(
                count,
                6,
                Some((3440.0, 1440.0)),
                true,
                DEFAULT_CARD_SCALE,
                DEFAULT_CARD_PADDING,
                true,
            );
            assert_eq!(layout.columns, expected_columns, "count={count}");
            assert!(
                (layout.metrics.scale - MAX_CARD_SCALE).abs() < 0.0001,
                "count={count}: huge monitor must max out card scale, got {}",
                layout.metrics.scale
            );
        }
    }

    #[test]
    fn dynamic_prefers_one_row_over_stacking_at_equal_scale() {
        let layout = picker_layout(
            2,
            6,
            Some((3440.0, 1440.0)),
            false,
            DEFAULT_CARD_SCALE,
            DEFAULT_CARD_PADDING,
            true,
        );
        assert_eq!(layout.columns, 2, "two windows must sit side by side");
    }

    #[test]
    fn fixed_mode_never_exceeds_configured_scale() {
        let counts = [1, 2, 3, 12, 30];
        for count in counts {
            let layout = picker_layout(
                count,
                6,
                Some((3440.0, 1440.0)),
                true,
                1.0,
                DEFAULT_CARD_PADDING,
                false,
            );
            assert!(
                layout.metrics.scale <= 1.0 + 0.0001,
                "count={count}: fixed mode grew past configured scale"
            );
        }
    }

    #[test]
    fn fixed_mode_shrinks_to_fit_instead_of_overshooting() {
        let layout = picker_layout(
            30,
            6,
            Some((1280.0, 800.0)),
            true,
            2.5,
            DEFAULT_CARD_PADDING,
            false,
        );
        assert!(
            layout.metrics.scale < 2.5,
            "30 windows at scale 2.5 on 1280x800 must shrink, got {}",
            layout.metrics.scale
        );
        assert!(content_height(&layout, 30, true) <= 800.0 * 0.9 + 0.01);
    }

    #[test]
    fn single_window_gets_single_column_panel() {
        let layout = picker_layout(
            1,
            6,
            Some((1920.0, 1080.0)),
            false,
            1.5,
            DEFAULT_CARD_PADDING,
            false,
        );
        assert_eq!(layout.columns, 1);
        assert!(layout.width <= width_for_cols(1, &layout.metrics) + WIDTH_SLACK);
    }

    #[test]
    fn capture_ceiling_covers_max_scale() {
        let m = CardMetrics::from_config(MAX_CARD_SCALE, DEFAULT_CARD_PADDING);
        assert!(PREVIEW_MAX_WIDTH as f32 >= m.preview_width);
        assert!(PREVIEW_MAX_HEIGHT as f32 >= m.preview_height);
    }
}
