use gpui::{point, px, size, Bounds};
use plugin_cli_sessions::placement::{parse_corner, Corner, CORNER_MARGIN};
use qol_gpui::placement::MonitorPlacement;

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
        assert_eq!(parse_corner(input), expected, "input: {input}");
    }
}

#[test]
fn the_panel_insets_from_its_configured_corner() {
    let monitor = Bounds::new(point(px(100.0), px(50.0)), size(px(1000.0), px(800.0)));
    let panel = size(px(340.0), px(400.0));

    let cases = [
        (Corner::TopLeft, 116.0, 66.0),
        (Corner::TopRight, 744.0, 66.0),
        (Corner::BottomLeft, 116.0, 434.0),
        (Corner::BottomRight, 744.0, 434.0),
    ];
    for (corner, expected_x, expected_y) in cases {
        let bounds = MonitorPlacement::corner(corner, CORNER_MARGIN).bounds(monitor, panel);
        assert_eq!(bounds.origin.x.to_f64(), expected_x, "{corner:?} x");
        assert_eq!(bounds.origin.y.to_f64(), expected_y, "{corner:?} y");
        assert_eq!(bounds.size.width.to_f64(), 340.0, "{corner:?} w");
        assert_eq!(bounds.size.height.to_f64(), 400.0, "{corner:?} h");
    }
}
