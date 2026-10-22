use gpui::{Bounds, Pixels, ResizeEdge};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct NativeRect {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

impl NativeRect {
    pub(crate) fn of_bounds(bounds: Bounds<Pixels>) -> Self {
        Self {
            x: bounds.origin.x.to_f64(),
            y: bounds.origin.y.to_f64(),
            width: bounds.size.width.to_f64(),
            height: bounds.size.height.to_f64(),
        }
    }

    fn right(self) -> f64 {
        self.x + self.width
    }

    fn bottom(self) -> f64 {
        self.y + self.height
    }

    fn spanning(left: f64, top: f64, right: f64, bottom: f64) -> Self {
        Self {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ResizeLimits {
    pub(crate) min_width: f64,
    pub(crate) min_height: f64,
    pub(crate) area: Option<NativeRect>,
}

#[derive(Clone, Copy)]
struct Sides {
    left: bool,
    top: bool,
    right: bool,
    bottom: bool,
}

fn dragged_sides(edge: ResizeEdge) -> Sides {
    let sides = |left, top, right, bottom| Sides {
        left,
        top,
        right,
        bottom,
    };
    match edge {
        ResizeEdge::Top => sides(false, true, false, false),
        ResizeEdge::Bottom => sides(false, false, false, true),
        ResizeEdge::Left => sides(true, false, false, false),
        ResizeEdge::Right => sides(false, false, true, false),
        ResizeEdge::TopLeft => sides(true, true, false, false),
        ResizeEdge::TopRight => sides(false, true, true, false),
        ResizeEdge::BottomLeft => sides(true, false, false, true),
        ResizeEdge::BottomRight => sides(false, false, true, true),
    }
}

pub(crate) fn resized_rect(
    start: NativeRect,
    edge: ResizeEdge,
    dx: f64,
    dy: f64,
    limits: ResizeLimits,
) -> NativeRect {
    let sides = dragged_sides(edge);
    let area = limits.area;
    let left = if sides.left {
        leading_edge(
            start.x + dx,
            area.map(|area| area.x.min(start.x)),
            start.right(),
            limits.min_width,
        )
    } else {
        start.x
    };
    let top = if sides.top {
        leading_edge(
            start.y + dy,
            area.map(|area| area.y.min(start.y)),
            start.bottom(),
            limits.min_height,
        )
    } else {
        start.y
    };
    let right = if sides.right {
        trailing_edge(
            start.right() + dx,
            area.map(|area| area.right().max(start.right())),
            start.x,
            limits.min_width,
        )
    } else {
        start.right()
    };
    let bottom = if sides.bottom {
        trailing_edge(
            start.bottom() + dy,
            area.map(|area| area.bottom().max(start.bottom())),
            start.y,
            limits.min_height,
        )
    } else {
        start.bottom()
    };
    NativeRect::spanning(left, top, right, bottom)
}

fn leading_edge(target: f64, floor: Option<f64>, opposite: f64, min_span: f64) -> f64 {
    let bounded = floor.map_or(target, |floor| target.max(floor));
    bounded.min(opposite - min_span)
}

fn trailing_edge(target: f64, ceiling: Option<f64>, opposite: f64, min_span: f64) -> f64 {
    let bounded = ceiling.map_or(target, |ceiling| target.min(ceiling));
    bounded.max(opposite + min_span)
}

#[cfg(test)]
mod tests {
    use super::{resized_rect, NativeRect, ResizeLimits};
    use gpui::ResizeEdge;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> NativeRect {
        NativeRect {
            x,
            y,
            width,
            height,
        }
    }

    fn limits(area: Option<NativeRect>) -> ResizeLimits {
        ResizeLimits {
            min_width: 200.0,
            min_height: 150.0,
            area,
        }
    }

    #[test]
    fn each_edge_moves_only_its_own_sides_within_the_minimum_and_the_work_area() {
        let start = rect(100.0, 100.0, 400.0, 300.0);
        let area = Some(rect(0.0, 0.0, 1000.0, 800.0));
        let cases = [
            (
                ResizeEdge::Right,
                50.0,
                0.0,
                rect(100.0, 100.0, 450.0, 300.0),
            ),
            (
                ResizeEdge::Right,
                -300.0,
                0.0,
                rect(100.0, 100.0, 200.0, 300.0),
            ),
            (
                ResizeEdge::Right,
                900.0,
                0.0,
                rect(100.0, 100.0, 900.0, 300.0),
            ),
            (
                ResizeEdge::Left,
                -50.0,
                0.0,
                rect(50.0, 100.0, 450.0, 300.0),
            ),
            (
                ResizeEdge::Left,
                300.0,
                0.0,
                rect(300.0, 100.0, 200.0, 300.0),
            ),
            (
                ResizeEdge::Left,
                -500.0,
                0.0,
                rect(0.0, 100.0, 500.0, 300.0),
            ),
            (ResizeEdge::Top, 0.0, -50.0, rect(100.0, 50.0, 400.0, 350.0)),
            (
                ResizeEdge::Top,
                0.0,
                200.0,
                rect(100.0, 250.0, 400.0, 150.0),
            ),
            (ResizeEdge::Top, 0.0, -400.0, rect(100.0, 0.0, 400.0, 400.0)),
            (
                ResizeEdge::Bottom,
                0.0,
                600.0,
                rect(100.0, 100.0, 400.0, 700.0),
            ),
            (
                ResizeEdge::Bottom,
                0.0,
                -200.0,
                rect(100.0, 100.0, 400.0, 150.0),
            ),
            (
                ResizeEdge::TopLeft,
                -30.0,
                -40.0,
                rect(70.0, 60.0, 430.0, 340.0),
            ),
            (
                ResizeEdge::TopRight,
                30.0,
                -40.0,
                rect(100.0, 60.0, 430.0, 340.0),
            ),
            (
                ResizeEdge::BottomLeft,
                -30.0,
                40.0,
                rect(70.0, 100.0, 430.0, 340.0),
            ),
            (
                ResizeEdge::BottomRight,
                30.0,
                40.0,
                rect(100.0, 100.0, 430.0, 340.0),
            ),
            (
                ResizeEdge::BottomRight,
                -900.0,
                -900.0,
                rect(100.0, 100.0, 200.0, 150.0),
            ),
            (
                ResizeEdge::TopLeft,
                900.0,
                900.0,
                rect(300.0, 250.0, 200.0, 150.0),
            ),
            (ResizeEdge::Right, 0.0, 500.0, start),
        ];
        for (edge, dx, dy, expected) in cases {
            assert_eq!(
                resized_rect(start, edge, dx, dy, limits(area)),
                expected,
                "{edge:?} dx={dx} dy={dy}"
            );
        }
    }

    #[test]
    fn without_a_known_work_area_only_the_minimum_binds() {
        let start = rect(100.0, 100.0, 400.0, 300.0);
        assert_eq!(
            resized_rect(start, ResizeEdge::BottomRight, 2000.0, 1000.0, limits(None)),
            rect(100.0, 100.0, 2400.0, 1300.0)
        );
        assert_eq!(
            resized_rect(start, ResizeEdge::TopLeft, -500.0, -500.0, limits(None)),
            rect(-400.0, -400.0, 900.0, 800.0)
        );
    }

    #[test]
    fn a_window_already_past_the_work_area_keeps_its_reach_but_gains_no_more() {
        let start = rect(900.0, 100.0, 400.0, 300.0);
        let area = Some(rect(0.0, 0.0, 1000.0, 800.0));
        assert_eq!(
            resized_rect(start, ResizeEdge::Right, 50.0, 0.0, limits(area)),
            start
        );
        assert_eq!(
            resized_rect(start, ResizeEdge::Right, -10.0, 0.0, limits(area)),
            rect(900.0, 100.0, 390.0, 300.0)
        );
    }

    #[test]
    fn the_minimum_wins_over_a_work_area_too_small_for_it() {
        let start = rect(0.0, 0.0, 300.0, 200.0);
        let area = Some(rect(0.0, 0.0, 250.0, 180.0));
        assert_eq!(
            resized_rect(start, ResizeEdge::BottomRight, -200.0, -200.0, limits(area)),
            rect(0.0, 0.0, 200.0, 150.0)
        );
        let narrow = rect(0.0, 0.0, 120.0, 100.0);
        assert_eq!(
            resized_rect(narrow, ResizeEdge::BottomRight, -10.0, -10.0, limits(area)),
            rect(0.0, 0.0, 200.0, 150.0)
        );
    }
}
