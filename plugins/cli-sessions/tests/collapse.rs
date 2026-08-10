use gpui::{point, px, size, Bounds};
use plugin_cli_sessions::collapse::{strip_bounds, CollapseState, STRIP_HEIGHT};
use plugin_cli_sessions::placement::Corner;
use proptest::prelude::*;

fn expanded_bounds() -> Bounds<gpui::Pixels> {
    Bounds::new(point(px(120.0), px(80.0)), size(px(360.0), px(400.0)))
}

#[test]
fn strip_bounds_keeps_width_and_anchors_to_the_expanded_corner() {
    let expanded = expanded_bounds();
    let bottom_y = 80.0 + 400.0 - f64::from(STRIP_HEIGHT);
    let cases = [
        (Corner::TopLeft, 120.0, 80.0),
        (Corner::TopRight, 120.0, 80.0),
        (Corner::BottomLeft, 120.0, bottom_y),
        (Corner::BottomRight, 120.0, bottom_y),
    ];
    for (corner, ex, ey) in cases {
        let b = strip_bounds(expanded, corner, STRIP_HEIGHT);
        assert_eq!(b.origin.x.to_f64(), ex, "{corner:?} x");
        assert_eq!(b.origin.y.to_f64(), ey, "{corner:?} y");
        assert_eq!(b.size.width.to_f64(), 360.0, "{corner:?} w");
        assert_eq!(
            b.size.height.to_f64(),
            f64::from(STRIP_HEIGHT),
            "{corner:?} h"
        );
    }
}

#[test]
fn strip_bounds_honors_an_explicit_strip_height() {
    let expanded = expanded_bounds();
    let strip = strip_bounds(expanded, Corner::BottomRight, 34.0);
    assert_eq!(strip.size.height.to_f64(), 34.0);
    assert_eq!(strip.origin.y.to_f64(), 80.0 + 400.0 - 34.0);
}

#[test]
fn collapse_remembers_expanded_bounds_and_returns_strip_bounds() {
    let mut state = CollapseState::new(Corner::TopRight);
    let expanded = expanded_bounds();
    let strip = state.collapse(expanded);
    assert!(state.is_collapsed());
    assert_eq!(strip.size.height.to_f64(), f64::from(STRIP_HEIGHT));
    let restored = state.expand().expect("expand restores bounds");
    assert_eq!(restored, expanded);
    assert!(!state.is_collapsed());
}

#[test]
fn expand_without_collapse_is_a_no_op() {
    let mut state = CollapseState::new(Corner::TopLeft);
    assert!(!state.is_collapsed());
    assert!(state.expand().is_none());
    assert!(!state.is_collapsed());
}

#[test]
fn second_expand_returns_nothing() {
    let mut state = CollapseState::new(Corner::BottomRight);
    let expanded = Bounds::new(point(px(10.0), px(20.0)), size(px(300.0), px(200.0)));
    state.collapse(expanded);
    assert!(state.expand().is_some());
    assert!(state.expand().is_none());
    assert!(!state.is_collapsed());
}

#[test]
fn collapse_replaces_the_remembered_bounds() {
    let mut state = CollapseState::new(Corner::BottomLeft);
    let first = Bounds::new(point(px(1.0), px(2.0)), size(px(300.0), px(200.0)));
    let second = Bounds::new(point(px(9.0), px(8.0)), size(px(350.0), px(250.0)));
    state.collapse(first);
    state.collapse(second);
    assert_eq!(state.expand().expect("restores latest"), second);
}

#[test]
fn open_while_collapsed_restores_the_expanded_bounds_exactly() {
    let mut state = CollapseState::new(Corner::TopLeft);
    let expanded = expanded_bounds();
    let strip = state.collapse(expanded);
    assert_ne!(strip, expanded);
    let shown = state.expand().expect("open expands");
    assert_eq!(shown, expanded);
    assert!(!state.is_collapsed());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn prop_strip_bounds_preserve_width_and_anchor_the_corner(
        x in -2000f32..8000f32,
        y in -2000f32..8000f32,
        w in 100f32..2000f32,
        h in 100f32..2000f32,
        strip_h in 20f32..60f32,
        corner in prop::sample::select(vec![
            Corner::TopLeft,
            Corner::TopRight,
            Corner::BottomLeft,
            Corner::BottomRight,
        ]),
    ) {
        let expanded = Bounds::new(point(px(x), px(y)), size(px(w), px(h)));
        let strip = strip_bounds(expanded, corner, strip_h);
        prop_assert_eq!(strip.size.width, expanded.size.width);
        prop_assert_eq!(strip.size.height, px(strip_h));
        prop_assert_eq!(strip.origin.x, expanded.origin.x);
        match corner {
            Corner::TopLeft | Corner::TopRight => {
                prop_assert_eq!(strip.origin.y, expanded.origin.y)
            }
            Corner::BottomLeft | Corner::BottomRight => {
                prop_assert_eq!(
                    strip.origin.y,
                    expanded.origin.y + expanded.size.height - px(strip_h)
                )
            }
        }
    }
}
