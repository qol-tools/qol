use std::time::{Duration, Instant};

use gpui::{point, px, size, ResizeEdge};

use super::{zone_at, ResizeFrame, Sizing, USER_SIZE_SETTLE};

#[test]
fn only_a_live_or_just_finished_drag_counts_as_user_sizing() {
    let now = Instant::now();
    let cases = [
        (Sizing::Idle, false, "no drag"),
        (Sizing::Dragging, true, "a live drag"),
        (Sizing::Settling(now), true, "the last bounds of a drag"),
        (
            Sizing::Settling(now - USER_SIZE_SETTLE - Duration::from_millis(1)),
            false,
            "a later window manager resize",
        ),
    ];
    for (sizing, expected, note) in cases {
        assert_eq!(sizing.counts_as_user(now), expected, "{note}");
    }
}

#[test]
fn a_drag_handed_to_the_window_manager_stops_counting_without_a_release() {
    let frame = ResizeFrame::new(String::new, size(px(200.0), px(150.0)));
    frame.state.start_drag();
    assert!(frame.is_user_sizing());
    frame
        .state
        .hand_off(Instant::now() - USER_SIZE_SETTLE - Duration::from_millis(1));
    assert!(!frame.is_user_sizing());
    frame.end();
    assert!(!frame.is_user_sizing());
}

#[test]
fn each_zone_covers_its_band_and_nothing_inside_it() {
    let viewport = size(px(400.0), px(300.0));
    let cases = [
        (0.0, 0.0, Some(ResizeEdge::TopLeft)),
        (9.0, 9.0, Some(ResizeEdge::TopLeft)),
        (399.0, 0.0, Some(ResizeEdge::TopRight)),
        (390.0, 9.0, Some(ResizeEdge::TopRight)),
        (0.0, 299.0, Some(ResizeEdge::BottomLeft)),
        (399.0, 299.0, Some(ResizeEdge::BottomRight)),
        (390.0, 290.0, Some(ResizeEdge::BottomRight)),
        (200.0, 0.0, Some(ResizeEdge::Top)),
        (200.0, 4.0, Some(ResizeEdge::Top)),
        (200.0, 5.0, None),
        (200.0, 295.0, Some(ResizeEdge::Bottom)),
        (200.0, 294.0, None),
        (0.0, 150.0, Some(ResizeEdge::Left)),
        (4.0, 150.0, Some(ResizeEdge::Left)),
        (5.0, 150.0, None),
        (395.0, 150.0, Some(ResizeEdge::Right)),
        (394.0, 150.0, None),
        (10.0, 4.0, Some(ResizeEdge::Top)),
        (4.0, 10.0, Some(ResizeEdge::Left)),
        (200.0, 150.0, None),
        (-1.0, 150.0, None),
        (400.0, 150.0, None),
        (200.0, 300.0, None),
    ];
    for (x, y, expected) in cases {
        assert_eq!(zone_at(point(px(x), px(y)), viewport), expected, "{x},{y}");
    }
}
