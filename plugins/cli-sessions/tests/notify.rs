use plugin_cli_sessions::notify::{announces_attention, Notice};
use plugin_cli_sessions::status::Status;
use qol_terminal_sessions::cli::{claude_tool, generic_tool};

#[test]
fn announces_only_on_transition_into_attention() {
    use Status::*;
    let cases = [
        (Working, NeedsYou, true),
        (Unknown, YourTurn, true),
        (Working, YourTurn, true),
        (NeedsYou, NeedsYou, false),
        (YourTurn, YourTurn, false),
        (YourTurn, Acknowledged, false),
        (Working, Service, false),
        (NeedsYou, Working, false),
    ];
    for (prev, new, expected) in cases {
        assert_eq!(
            announces_attention(prev, new),
            expected,
            "{prev:?} -> {new:?}"
        );
    }
}

#[test]
fn notice_prefixes_body_with_tool_only_for_agents() {
    let claude = Notice::new(&claude_tool(), "improve-logging".to_string(), "needs you");
    assert_eq!(claude.title, "improve-logging");
    assert_eq!(claude.body, "Claude \u{00B7} needs you");

    let generic = Notice::new(&generic_tool(), "qol dev".to_string(), "your turn");
    assert_eq!(generic.body, "your turn", "generic carries no tool prefix");
}
