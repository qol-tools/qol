use plugin_cli_sessions::nav::next_attention;
use plugin_cli_sessions::status::Status;

#[test]
fn next_attention_cycles_over_attention_rows_only() {
    use Status::*;
    let rows = [NeedsYou, YourTurn, Working, Service, Unknown];
    let cases = [
        (0usize, Some(1usize)),
        (1, Some(0)),
        (2, Some(0)),
        (4, Some(0)),
    ];
    for (current, expected) in cases {
        assert_eq!(
            next_attention(&rows, current),
            expected,
            "current: {current}"
        );
    }
}

#[test]
fn next_attention_handles_empty_none_and_single() {
    use Status::*;
    assert_eq!(next_attention(&[], 0), None, "empty list");
    assert_eq!(
        next_attention(&[Working, Unknown], 0),
        None,
        "no attention rows present"
    );
    assert_eq!(
        next_attention(&[NeedsYou, Working], 0),
        Some(0),
        "the only attention row wraps back to itself"
    );
}
