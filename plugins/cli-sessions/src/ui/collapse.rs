use gpui::{point, px, size, Bounds, Pixels, Point, ResizeEdge, Size};
use qol_gpui::resize_frame;

use crate::ui::placement::Corner;

pub const STRIP_HEIGHT: f32 = qol_gpui::theme::HEIGHT_SETTING_ROW;
pub const MIN_WIDTH: f32 = 360.0;
pub const MIN_LIST_ROWS: f32 = 3.0;
const BOUNDS_TOLERANCE: f32 = 1.0;

pub fn unfolded_min_size() -> Size<Pixels> {
    let list = MIN_LIST_ROWS * qol_gpui::theme::LIST_ENTRY_HEIGHTS[1];
    size(
        px(MIN_WIDTH),
        px(STRIP_HEIGHT + list + qol_gpui::theme::HEIGHT_HINT_BAR),
    )
}

pub fn folded_min_size() -> Size<Pixels> {
    size(px(MIN_WIDTH), px(STRIP_HEIGHT))
}

pub fn unfolded_bounds(
    expanded: Bounds<Pixels>,
    strip: Bounds<Pixels>,
    corner: Corner,
) -> Bounds<Pixels> {
    reanchor_expanded(
        Bounds::new(
            expanded.origin,
            size(strip.size.width, expanded.size.height),
        ),
        strip,
        corner,
    )
}

pub fn folded_height_drifted(height: Pixels) -> bool {
    (height - px(STRIP_HEIGHT)).abs() > px(BOUNDS_TOLERANCE)
}

pub fn bounds_differ(left: Bounds<Pixels>, right: Bounds<Pixels>) -> bool {
    let far = |a: Pixels, b: Pixels| (a - b).abs() > px(BOUNDS_TOLERANCE);
    far(left.origin.x, right.origin.x)
        || far(left.origin.y, right.origin.y)
        || far(left.size.width, right.size.width)
        || far(left.size.height, right.size.height)
}

pub fn presses_height_edge(position: Point<Pixels>, viewport: Size<Pixels>) -> bool {
    resize_frame::edge_at(position, viewport)
        .is_some_and(|edge| !matches!(edge, ResizeEdge::Left | ResizeEdge::Right))
}

pub fn presses_edge(position: Point<Pixels>, viewport: Size<Pixels>) -> bool {
    resize_frame::edge_at(position, viewport).is_some()
}

pub fn strip_bounds(expanded: Bounds<Pixels>, corner: Corner, strip_height: f32) -> Bounds<Pixels> {
    let y = match corner {
        Corner::TopLeft | Corner::TopRight => expanded.origin.y,
        Corner::BottomLeft | Corner::BottomRight => {
            expanded.origin.y + expanded.size.height - px(strip_height)
        }
    };
    Bounds::new(
        point(expanded.origin.x, y),
        size(expanded.size.width, px(strip_height)),
    )
}

pub fn reanchor_expanded(
    expanded: Bounds<Pixels>,
    strip: Bounds<Pixels>,
    corner: Corner,
) -> Bounds<Pixels> {
    let x = match corner {
        Corner::TopLeft | Corner::BottomLeft => strip.origin.x,
        Corner::TopRight | Corner::BottomRight => {
            strip.origin.x + strip.size.width - expanded.size.width
        }
    };
    let y = match corner {
        Corner::TopLeft | Corner::TopRight => strip.origin.y,
        Corner::BottomLeft | Corner::BottomRight => {
            strip.origin.y + strip.size.height - expanded.size.height
        }
    };
    Bounds::new(point(x, y), expanded.size)
}

pub struct CollapseState {
    corner: Corner,
    collapsed: bool,
    expanded: Option<Bounds<Pixels>>,
}

impl CollapseState {
    pub fn new(corner: Corner) -> Self {
        Self {
            corner,
            collapsed: false,
            expanded: None,
        }
    }

    pub fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    pub fn corner(&self) -> Corner {
        self.corner
    }

    pub fn collapse(&mut self, expanded: Bounds<Pixels>) -> Bounds<Pixels> {
        self.collapsed = true;
        self.expanded = Some(expanded);
        strip_bounds(expanded, self.corner, STRIP_HEIGHT)
    }

    pub fn unfolded(&self, strip: Bounds<Pixels>) -> Option<Bounds<Pixels>> {
        self.expanded
            .map(|expanded| unfolded_bounds(expanded, strip, self.corner))
    }

    pub fn expand(&mut self) -> Option<Bounds<Pixels>> {
        let expanded = self.expanded.take()?;
        self.collapsed = false;
        Some(expanded)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        bounds_differ, folded_height_drifted, folded_min_size, presses_edge, presses_height_edge,
        unfolded_bounds, unfolded_min_size, CollapseState, STRIP_HEIGHT,
    };
    use crate::ui::placement::Corner;
    use gpui::{point, px, size, Bounds, Pixels};
    use qol_gpui::resize_frame;

    const CORNERS: [Corner; 4] = [
        Corner::TopLeft,
        Corner::TopRight,
        Corner::BottomLeft,
        Corner::BottomRight,
    ];

    fn bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
    }

    #[test]
    fn the_minimum_holds_the_header_three_rows_and_the_hint_bar() {
        assert_eq!(
            unfolded_min_size(),
            size(px(360.0), px(52.0 + 3.0 * 40.0 + 40.0))
        );
        assert_eq!(folded_min_size(), size(px(360.0), px(STRIP_HEIGHT)));
    }

    #[test]
    fn unfolding_keeps_the_strip_width_and_the_unfolded_height_at_every_corner() {
        let expanded = bounds(1000.0, 16.0, 360.0, 640.0);
        let strip = bounds(940.0, 300.0, 480.0, STRIP_HEIGHT);
        let cases = [
            (Corner::TopLeft, 940.0, 300.0),
            (Corner::TopRight, 940.0, 300.0),
            (Corner::BottomLeft, 940.0, 300.0 + STRIP_HEIGHT - 640.0),
            (Corner::BottomRight, 940.0, 300.0 + STRIP_HEIGHT - 640.0),
        ];
        for (corner, x, y) in cases {
            assert_eq!(
                unfolded_bounds(expanded, strip, corner),
                bounds(x, y, 480.0, 640.0),
                "{corner:?}"
            );
        }
    }

    #[test]
    fn an_untouched_strip_unfolds_to_exactly_the_bounds_it_folded_from() {
        let expanded = bounds(1544.0, 16.0, 360.0, 712.0);
        for corner in CORNERS {
            let mut state = CollapseState::new(corner);
            let strip = state.collapse(expanded);
            assert_eq!(state.unfolded(strip), Some(expanded), "{corner:?}");
        }
    }

    #[test]
    fn only_a_folded_panel_has_unfolded_bounds() {
        let expanded = bounds(16.0, 16.0, 360.0, 400.0);
        let mut state = CollapseState::new(Corner::TopRight);
        assert_eq!(state.unfolded(expanded), None);
        let strip = state.collapse(expanded);
        assert!(state.unfolded(strip).is_some());
        state.expand();
        assert_eq!(state.unfolded(strip), None);
    }

    #[test]
    fn a_folded_height_drifts_only_past_the_tolerance() {
        for (height, drifted) in [
            (STRIP_HEIGHT, false),
            (STRIP_HEIGHT + 0.5, false),
            (STRIP_HEIGHT - 1.0, false),
            (STRIP_HEIGHT + 2.0, true),
            (STRIP_HEIGHT - 2.0, true),
            (400.0, true),
        ] {
            assert_eq!(folded_height_drifted(px(height)), drifted, "{height}");
        }
    }

    #[test]
    fn bounds_differ_past_a_pixel_on_any_side() {
        let base = bounds(10.0, 20.0, 360.0, 400.0);
        assert!(!bounds_differ(base, base));
        assert!(!bounds_differ(base, bounds(10.5, 20.0, 360.0, 400.5)));
        for moved in [
            bounds(12.0, 20.0, 360.0, 400.0),
            bounds(10.0, 18.0, 360.0, 400.0),
            bounds(10.0, 20.0, 380.0, 400.0),
            bounds(10.0, 20.0, 360.0, 52.0),
        ] {
            assert!(bounds_differ(base, moved), "{moved:?}");
        }
    }

    #[test]
    fn a_press_in_the_frame_top_or_bottom_band_reaches_a_height_edge() {
        let strip = size(px(360.0), px(STRIP_HEIGHT));
        let band = resize_frame::EDGE_BAND;
        for (x, y, height_edge, any_edge) in [
            (180.0, 2.0, true, true),
            (180.0, band - 1.0, true, true),
            (180.0, band, false, false),
            (180.0, STRIP_HEIGHT - 2.0, true, true),
            (180.0, STRIP_HEIGHT - band - 1.0, false, false),
            (9.0, 9.0, true, true),
            (2.0, 2.0, true, true),
            (358.0, STRIP_HEIGHT - 2.0, true, true),
            (180.0, 26.0, false, false),
            (2.0, 26.0, false, true),
            (358.0, 26.0, false, true),
        ] {
            let at = point(px(x), px(y));
            let shown = resize_frame::FRAME_RESIZES_WINDOW;
            assert_eq!(
                presses_height_edge(at, strip),
                height_edge && shown,
                "{x},{y}"
            );
            assert_eq!(presses_edge(at, strip), any_edge && shown, "{x},{y}");
        }
    }
}
