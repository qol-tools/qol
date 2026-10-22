use std::time::{Duration, Instant};

use gpui::{px, size, ResizeEdge};

use super::{
    drawn_edges, resize_path, sizing_ended, ResizeFrame, ResizePath, Sizing, HAND_OFF_LIMIT,
    USER_SIZE_SETTLE,
};

fn frame() -> ResizeFrame {
    ResizeFrame::new(String::new, size(px(200.0), px(150.0)))
}

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
        (Sizing::HandedOff(now), true, "a fresh hand-off"),
        (
            Sizing::HandedOff(now - HAND_OFF_LIMIT - Duration::from_millis(1)),
            false,
            "a hand-off past its limit",
        ),
    ];
    for (sizing, expected, note) in cases {
        assert_eq!(sizing.counts_as_user(now), expected, "{note}");
    }
}

#[test]
fn a_drag_handed_to_the_window_manager_counts_until_its_limit_without_a_release() {
    let frame = frame();
    frame.state.start_drag();
    assert!(frame.is_user_sizing());
    frame
        .state
        .hand_off(Instant::now() - USER_SIZE_SETTLE - Duration::from_millis(1));
    assert!(frame.is_user_sizing());
    frame
        .state
        .hand_off(Instant::now() - HAND_OFF_LIMIT - Duration::from_millis(1));
    assert!(!frame.is_user_sizing());
    assert_eq!(frame.state.release(Instant::now()), None);
    assert!(!frame.is_user_sizing());
}

#[test]
fn a_release_settles_only_a_live_drag_without_client_state() {
    let now = Instant::now();
    let cases = [
        (Sizing::Idle, None, "no drag"),
        (
            Sizing::Dragging,
            Some(Sizing::Settling(now)),
            "a drag whose press never reached the client path",
        ),
        (
            Sizing::HandedOff(now - USER_SIZE_SETTLE),
            Some(Sizing::Settling(now)),
            "a live hand-off",
        ),
        (
            Sizing::HandedOff(now - HAND_OFF_LIMIT - Duration::from_millis(1)),
            None,
            "an expired hand-off",
        ),
        (Sizing::Settling(now), None, "a drag already settling"),
    ];
    for (sizing, expected, note) in cases {
        let frame = frame();
        frame.state.sizing.set(sizing);
        let epoch = frame.state.epoch.get();
        assert_eq!(frame.state.release(now), expected, "{note}");
        assert_eq!(
            frame.state.sizing.get(),
            expected.unwrap_or(sizing),
            "{note}"
        );
        assert_eq!(frame.state.epoch.get(), epoch, "{note}");
    }
}

#[test]
fn a_finish_settles_only_the_drag_of_the_current_epoch() {
    let now = Instant::now();
    let frame = frame();
    let stale = frame.state.start_drag();
    let current = frame.state.start_drag();
    assert!(!frame.state.finish(stale, now));
    assert_eq!(frame.state.sizing.get(), Sizing::Dragging);
    assert!(frame.state.finish(current, now));
    assert_eq!(frame.state.sizing.get(), Sizing::Settling(now));
    assert_ne!(frame.state.epoch.get(), current);
    assert!(!frame.state.finish(current, now));
}

#[test]
fn ending_user_sizing_mid_drag_stops_the_drag_and_its_settle() {
    let now = Instant::now();
    let frame = frame();
    let epoch = frame.state.start_drag();
    frame.end_user_sizing();
    assert!(!frame.is_user_sizing());
    assert!(frame.state.drag.borrow().is_none());
    assert!(!frame.state.finish(epoch, now));
    assert_eq!(frame.state.sizing.get(), Sizing::Idle);

    let settled = frame.state.start_drag();
    assert!(frame.state.finish(settled, now));
    let pending = frame.state.epoch.get();
    frame.end_user_sizing();
    assert!(!frame.state.take_ended(Sizing::Settling(now), pending));
}

#[test]
fn the_sizing_ended_signal_fires_once_per_drag_and_only_for_its_own_transition() {
    let now = Instant::now();
    let later = now + Duration::from_millis(1);
    let cases = [
        (
            Sizing::Settling(now),
            2,
            Sizing::Settling(now),
            2,
            true,
            "a settled drag",
        ),
        (
            Sizing::HandedOff(now),
            1,
            Sizing::HandedOff(now),
            1,
            true,
            "an expired hand-off",
        ),
        (
            Sizing::Settling(later),
            1,
            Sizing::HandedOff(now),
            1,
            false,
            "a hand-off the release already settled",
        ),
        (
            Sizing::Dragging,
            3,
            Sizing::Settling(now),
            2,
            false,
            "a newer drag",
        ),
        (
            Sizing::Idle,
            3,
            Sizing::Settling(now),
            2,
            false,
            "a programmatic size",
        ),
        (
            Sizing::Idle,
            2,
            Sizing::Settling(now),
            2,
            false,
            "a signal that already fired",
        ),
    ];
    for (current, epoch_now, scheduled, epoch_then, expected, note) in cases {
        assert_eq!(
            sizing_ended(current, epoch_now, scheduled, epoch_then),
            expected,
            "{note}"
        );
    }

    let frame = frame();
    let epoch = frame.state.start_drag();
    assert!(frame.state.finish(epoch, now));
    let settled = frame.state.epoch.get();
    assert!(frame.state.take_ended(Sizing::Settling(now), settled));
    assert!(!frame.is_user_sizing());
    assert!(!frame.state.take_ended(Sizing::Settling(now), settled));
}

#[test]
fn only_a_client_resized_frame_skips_the_window_manager() {
    let cases = [
        (true, false, ResizePath::WindowManager, "a Linux panel"),
        (true, true, ResizePath::Client, "a Linux dock window"),
        (false, false, ResizePath::Client, "a Windows panel"),
        (
            false,
            true,
            ResizePath::Client,
            "a Windows client-resized window",
        ),
    ];
    for (by_window_manager, client_resized, expected, note) in cases {
        assert_eq!(
            resize_path(by_window_manager, client_resized),
            expected,
            "{note}"
        );
    }
}

#[test]
fn a_fixed_height_frame_draws_only_the_side_zones() {
    assert_eq!(drawn_edges(false).count(), 8);
    assert_eq!(
        drawn_edges(true).collect::<Vec<_>>(),
        vec![ResizeEdge::Left, ResizeEdge::Right]
    );
}
