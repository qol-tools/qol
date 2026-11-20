use std::cell::Cell;

use super::*;

fn run(enabled: bool, push_result: bool, os_result: bool) -> (bool, u32, u32) {
    let pushes = Cell::new(0);
    let shown = Cell::new(0);
    let delivered = deliver_in_order(
        enabled,
        &Notification::new("title", "body"),
        |_| {
            pushes.set(pushes.get() + 1);
            push_result
        },
        |_| {
            shown.set(shown.get() + 1);
            os_result
        },
    );
    (delivered, pushes.get(), shown.get())
}

#[test]
fn the_send_order_covers_every_branch() {
    let cases = [
        (false, true, true, (false, 0, 0)),
        (false, false, false, (false, 0, 0)),
        (true, true, true, (true, 1, 0)),
        (true, true, false, (true, 1, 0)),
        (true, false, true, (true, 1, 1)),
        (true, false, false, (false, 1, 1)),
    ];
    for (enabled, push, os, expected) in cases {
        assert_eq!(
            run(enabled, push, os),
            expected,
            "enabled={enabled} push={push} os={os}"
        );
    }
}

#[test]
fn both_steps_receive_the_notification() {
    let seen = Cell::new(0);
    let notification = Notification::new("title", "body").key("k");
    deliver_in_order(
        true,
        &notification,
        |sent| {
            assert_eq!(sent.title, "title");
            assert_eq!(sent.meta.key.as_deref(), Some("k"));
            seen.set(seen.get() + 1);
            false
        },
        |sent| {
            assert_eq!(sent.body, "body");
            seen.set(seen.get() + 1);
            true
        },
    );
    assert_eq!(seen.get(), 2);
}
