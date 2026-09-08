use gpui::*;

pub fn at(bounds: Bounds<Pixels>, x: f32, y: f32) -> Point<Pixels> {
    bounds.origin + point(px(x), px(y))
}

pub fn from_normalized(bounds: Bounds<Pixels>, normalized: Point<f32>) -> Point<Pixels> {
    bounds.origin
        + point(
            bounds.size.width * normalized.x,
            bounds.size.height * normalized.y,
        )
}

pub fn to_normalized(bounds: Bounds<Pixels>, position: Point<Pixels>) -> Option<Point<f32>> {
    if bounds.size.width <= px(0.0) || bounds.size.height <= px(0.0) {
        return None;
    }
    Some(point(
        (position.x - bounds.origin.x) / bounds.size.width,
        (position.y - bounds.origin.y) / bounds.size.height,
    ))
}

#[cfg(test)]
mod tests {
    use super::{at, from_normalized, to_normalized};
    use gpui::{point, px, size, Bounds};

    #[test]
    fn at_offsets_canvas_coordinates_by_the_bounds_origin() {
        let bounds = Bounds::new(point(px(10.0), px(20.0)), size(px(100.0), px(50.0)));
        assert_eq!(at(bounds, 5.0, 7.0), point(px(15.0), px(27.0)));
    }

    #[test]
    fn normalized_positions_round_trip_through_window_space() {
        let bounds = Bounds::new(point(px(8.0), px(16.0)), size(px(64.0), px(32.0)));
        let normalized = point(0.25, 0.5);
        let window = from_normalized(bounds, normalized);
        assert_eq!(to_normalized(bounds, window), Some(normalized));
    }

    #[test]
    fn zero_sized_bounds_have_no_normalized_position() {
        let cases = [
            Bounds::new(point(px(0.0), px(0.0)), size(px(0.0), px(0.0))),
            Bounds::new(point(px(4.0), px(4.0)), size(px(0.0), px(32.0))),
            Bounds::new(point(px(4.0), px(4.0)), size(px(32.0), px(0.0))),
        ];
        for bounds in cases {
            assert_eq!(to_normalized(bounds, point(px(1.0), px(1.0))), None);
        }
    }
}
