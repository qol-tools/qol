use std::time::{Duration, Instant};

use super::{Sizing, USER_SIZE_SETTLE};

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
