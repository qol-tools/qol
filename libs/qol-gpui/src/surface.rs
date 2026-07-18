use gpui::*;

pub const CORNER_MARGIN: f32 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

#[derive(Clone, Copy, Debug)]
pub enum Anchor {
    CornerStack(Corner),
}

fn corner_anchored_bounds(
    monitor: Bounds<Pixels>,
    corner: Corner,
    win: Size<Pixels>,
    margin: f32,
) -> Bounds<Pixels> {
    let min_x = monitor.origin.x.to_f64() as f32 + margin;
    let max_x =
        ((monitor.origin.x + monitor.size.width - win.width).to_f64() as f32 - margin).max(min_x);
    let min_y = monitor.origin.y.to_f64() as f32 + margin;
    let max_y =
        ((monitor.origin.y + monitor.size.height - win.height).to_f64() as f32 - margin).max(min_y);
    let x = match corner {
        Corner::TopLeft | Corner::BottomLeft => min_x,
        Corner::TopRight | Corner::BottomRight => max_x,
    };
    let y = match corner {
        Corner::TopLeft | Corner::TopRight => min_y,
        Corner::BottomLeft | Corner::BottomRight => max_y,
    };
    Bounds::new(point(px(x), px(y)), win)
}

#[cfg(test)]
mod tests {
    use super::{corner_anchored_bounds, Corner};
    use gpui::{point, px, size, Bounds};

    #[test]
    fn corner_anchored_bounds_places_each_corner_inside_margins() {
        let monitor = Bounds::new(point(px(1920.0), px(0.0)), size(px(2560.0), px(1440.0)));
        let win = size(px(340.0), px(76.0));
        let cases = [
            (Corner::TopLeft, (1944.0, 24.0)),
            (Corner::TopRight, (4116.0, 24.0)),
            (Corner::BottomLeft, (1944.0, 1340.0)),
            (Corner::BottomRight, (4116.0, 1340.0)),
        ];

        for (corner, expected) in cases {
            let bounds = corner_anchored_bounds(monitor, corner, win, 24.0);
            assert_eq!(
                (
                    bounds.origin.x.to_f64() as f32,
                    bounds.origin.y.to_f64() as f32
                ),
                expected,
                "corner: {corner:?}"
            );
        }
    }

    #[test]
    fn corner_anchored_bounds_supports_negative_origins_and_tiny_monitors() {
        let win = size(px(340.0), px(76.0));

        let negative = corner_anchored_bounds(
            Bounds::new(point(px(-1920.0), px(-200.0)), size(px(1920.0), px(1080.0))),
            Corner::BottomRight,
            win,
            24.0,
        );
        assert_eq!(negative.origin.x.to_f64(), -364.0);
        assert_eq!(negative.origin.y.to_f64(), 780.0);

        let tiny = corner_anchored_bounds(
            Bounds::new(point(px(0.0), px(0.0)), size(px(200.0), px(50.0))),
            Corner::BottomRight,
            win,
            24.0,
        );
        assert_eq!(tiny.origin.x.to_f64(), 24.0);
        assert_eq!(tiny.origin.y.to_f64(), 24.0);
    }
}
