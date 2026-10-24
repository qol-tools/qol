use std::collections::HashSet;
use std::hash::Hash;

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

pub fn cursor_row(touched: &[usize], under: Option<usize>) -> Option<usize> {
    under.filter(|row| touched.contains(row))
}

pub fn merged<T: Eq + Hash>(
    base: impl IntoIterator<Item = T>,
    touched: impl IntoIterator<Item = T>,
    additive: bool,
) -> HashSet<T> {
    base.into_iter()
        .filter(|_| additive)
        .chain(touched)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{content_point, cursor_row, edge_step, merged, outside};

    fn set(items: &[&'static str]) -> HashSet<&'static str> {
        items.iter().copied().collect()
    }

    #[test]
    fn a_box_adds_to_the_picks_it_started_with_or_replaces_them() {
        let cases = [
            (&["a", "b"][..], &["b", "c"][..], true, &["a", "b", "c"][..]),
            (&["a", "b"], &["b", "c"], false, &["b", "c"]),
            (&["a", "b"], &[], true, &["a", "b"]),
            (&["a", "b"], &[], false, &[]),
            (&[], &["x"], true, &["x"]),
            (&[], &["x"], false, &["x"]),
            (&[], &[], true, &[]),
            (&["a"], &["a", "a"], false, &["a"]),
            (&["a", "a"], &["b"], true, &["a", "b"]),
        ];
        for (base, touched, additive, want) in cases {
            let got = merged(base.iter().copied(), touched.iter().copied(), additive);
            assert_eq!(got, set(want), "{base:?} {touched:?} {additive}");
        }
    }

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

    #[test]
    fn the_cursor_follows_the_row_under_the_pointer_only_when_the_box_holds_it() {
        let cases = [
            (&[1, 2, 3][..], Some(2), Some(2)),
            (&[1, 2, 3], Some(1), Some(1)),
            (&[1, 2, 3], Some(3), Some(3)),
            (&[1, 2, 3], Some(5), None),
            (&[1, 2, 3], Some(0), None),
            (&[1, 2, 3], None, None),
            (&[], Some(0), None),
            (&[], None, None),
        ];
        for (touched, under, want) in cases {
            assert_eq!(cursor_row(touched, under), want, "{touched:?} {under:?}");
        }
    }
}
