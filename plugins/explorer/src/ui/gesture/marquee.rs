const EDGE_GAIN: f32 = 10.0;
const EDGE_SLOWEST: f32 = 120.0;
const EDGE_FASTEST: f32 = 1800.0;

pub fn content_point(pointer: (f32, f32), area: (f32, f32), scroll: f32) -> (f32, f32) {
    (pointer.0 - area.0, pointer.1 - area.1 + scroll)
}

pub fn outside(y: f32, top: f32, bottom: f32) -> f32 {
    if y < top {
        y - top
    } else if y > bottom {
        y - bottom
    } else {
        0.0
    }
}

pub fn edge_step(outside: f32, seconds: f32) -> f32 {
    if outside == 0.0 {
        return 0.0;
    }
    let speed = (outside.abs() * EDGE_GAIN).clamp(EDGE_SLOWEST, EDGE_FASTEST);
    speed * seconds * outside.signum()
}

#[cfg(test)]
mod tests {
    use super::{content_point, edge_step, outside};

    #[test]
    fn a_pointer_maps_into_content_coordinates_that_ignore_where_the_list_scrolled_to() {
        let cases = [
            ((110.0, 230.0), (100.0, 200.0), 0.0, (10.0, 30.0)),
            ((110.0, 230.0), (100.0, 200.0), 50.0, (10.0, 80.0)),
            ((100.0, 200.0), (100.0, 200.0), 1234.5, (0.0, 1234.5)),
            ((90.0, 150.0), (100.0, 200.0), 0.0, (-10.0, -50.0)),
            ((90.0, 150.0), (100.0, 200.0), 60.0, (-10.0, 10.0)),
        ];
        for (pointer, area, scroll, want) in cases {
            assert_eq!(
                content_point(pointer, area, scroll),
                want,
                "{pointer:?} {area:?} {scroll}"
            );
        }
    }

    #[test]
    fn a_pointer_that_holds_still_moves_down_content_by_exactly_the_scroll() {
        let first = content_point((150.0, 300.0), (100.0, 200.0), 40.0);
        assert_eq!(first, (50.0, 140.0));
        for scroll in [40.0, 100.0, 700.0] {
            let here = content_point((150.0, 300.0), (100.0, 200.0), scroll);
            assert_eq!(here.0, first.0, "{scroll}");
            assert_eq!(here.1 - first.1, scroll - 40.0, "{scroll}");
        }
    }

    #[test]
    fn outside_is_signed_distance_past_the_nearest_edge_and_zero_inside() {
        let cases = [
            (100.0, 0.0),
            (300.0, 0.0),
            (500.0, 0.0),
            (99.0, -1.0),
            (40.0, -60.0),
            (501.0, 1.0),
            (800.0, 300.0),
            (-20.0, -120.0),
        ];
        for (y, want) in cases {
            assert_eq!(outside(y, 100.0, 500.0), want, "{y}");
        }
    }

    #[test]
    fn edge_scrolling_speeds_up_with_distance_inside_a_floor_and_a_cap() {
        let cases = [
            (0.0, 0.016, 0.0),
            (1.0, 1.0, 120.0),
            (-1.0, 1.0, -120.0),
            (12.0, 1.0, 120.0),
            (50.0, 1.0, 500.0),
            (-50.0, 0.5, -250.0),
            (180.0, 1.0, 1800.0),
            (500.0, 1.0, 1800.0),
            (-5000.0, 1.0, -1800.0),
            (30.0, 0.0, 0.0),
            (30.0, 2.0, 600.0),
        ];
        for (distance, seconds, want) in cases {
            assert_eq!(edge_step(distance, seconds), want, "{distance} {seconds}");
        }
    }
}
