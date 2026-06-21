use plugin_cli_sessions::nav::next_attention;
use plugin_cli_sessions::status::Status;

#[test]
fn no_cursor_lands_on_the_highest_priority_attention_row() {
    use Status::*;
    // rows arrive already priority-sorted (NeedsYou, then YourTurn, by recency)
    let rows = [NeedsYou, NeedsYou, YourTurn, Working, Unknown];
    assert_eq!(
        next_attention(&rows, None),
        Some(0),
        "a fresh press jumps to the top of the priority order, not the second item"
    );
}

#[test]
fn cursor_advances_then_wraps_in_priority_order() {
    use Status::*;
    let rows = [NeedsYou, YourTurn, Working, Service, Unknown];
    let cases = [
        (Some(0usize), Some(1usize)),
        (Some(1), Some(0)),
        (Some(2), Some(0)),
        (Some(4), Some(0)),
    ];
    for (current, expected) in cases {
        assert_eq!(
            next_attention(&rows, current),
            expected,
            "current: {current:?}"
        );
    }
}

#[test]
fn empty_none_and_single() {
    use Status::*;
    assert_eq!(next_attention(&[], None), None, "empty list");
    assert_eq!(
        next_attention(&[Working, Unknown], None),
        None,
        "no attention rows present"
    );
    assert_eq!(
        next_attention(&[NeedsYou, Working], Some(0)),
        Some(0),
        "the only attention row wraps back to itself"
    );
}
