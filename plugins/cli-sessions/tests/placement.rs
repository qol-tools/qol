use gpui::{point, px, size, Bounds};
use plugin_cli_sessions::placement::{clamp_bounds, corner_bounds, Corner};

#[test]
fn corner_parse_falls_back_to_top_right() {
    let cases = [
        ("top-left", Corner::TopLeft),
        ("TOP_LEFT", Corner::TopLeft),
        ("bottom-left", Corner::BottomLeft),
        ("bottom-right", Corner::BottomRight),
        ("top-right", Corner::TopRight),
        ("garbage", Corner::TopRight),
        ("", Corner::TopRight),
    ];
    for (input, expected) in cases {
        assert_eq!(Corner::parse(input), expected, "input: {input}");
    }
}

#[test]
fn corner_bounds_insets_window_from_each_corner() {
    let monitor = Bounds::new(point(px(100.0), px(50.0)), size(px(1000.0), px(800.0)));
    let win = size(px(340.0), px(400.0));
    let margin = 16.0;

    let cases = [
        (Corner::TopLeft, 116.0, 66.0),
        (Corner::TopRight, 744.0, 66.0),
        (Corner::BottomLeft, 116.0, 434.0),
        (Corner::BottomRight, 744.0, 434.0),
    ];
    for (corner, ex, ey) in cases {
        let b = corner_bounds(monitor, win, corner, margin);
        assert_eq!(b.origin.x.to_f64(), ex, "{corner:?} x");
        assert_eq!(b.origin.y.to_f64(), ey, "{corner:?} y");
        assert_eq!(b.size.width.to_f64(), 340.0, "{corner:?} w");
        assert_eq!(b.size.height.to_f64(), 400.0, "{corner:?} h");
    }
}

#[test]
fn clamp_bounds_keeps_an_in_bounds_panel_untouched() {
    let monitor = Bounds::new(point(px(0.0), px(0.0)), size(px(1920.0), px(1080.0)));
    let bounds = Bounds::new(point(px(100.0), px(80.0)), size(px(360.0), px(400.0)));
    assert_eq!(clamp_bounds(monitor, bounds, 12.0), bounds);
}

#[test]
fn clamp_bounds_pulls_an_offscreen_panel_back_onto_the_monitor() {
    let monitor = Bounds::new(point(px(0.0), px(0.0)), size(px(1920.0), px(1080.0)));
    let below = Bounds::new(point(px(100.0), px(1100.0)), size(px(360.0), px(400.0)));
    let clamped = clamp_bounds(monitor, below, 12.0);
    assert_eq!(clamped.origin.y.to_f64(), 1080.0 - 400.0 - 12.0);
    let off_left = Bounds::new(point(px(-300.0), px(80.0)), size(px(360.0), px(400.0)));
    let clamped = clamp_bounds(monitor, off_left, 12.0);
    assert_eq!(clamped.origin.x.to_f64(), 12.0);
}

#[test]
fn clamp_bounds_pins_an_oversized_panel_to_the_min_corner() {
    let monitor = Bounds::new(point(px(0.0), px(0.0)), size(px(400.0), px(300.0)));
    let oversized = Bounds::new(point(px(900.0), px(900.0)), size(px(2000.0), px(1500.0)));
    let clamped = clamp_bounds(monitor, oversized, 8.0);
    assert_eq!(clamped.origin.x.to_f64(), 8.0);
    assert_eq!(clamped.origin.y.to_f64(), 8.0);
}
