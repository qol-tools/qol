use gpui::{point, px, size, Bounds};
use plugin_cli_sessions::placement::{corner_bounds, Corner};

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
