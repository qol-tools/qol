use qol_cli_sessions::notify::{announces_attention, Notice};
use qol_cli_sessions::status::Status;
use qol_terminal_sessions::cli::{claude_tool, generic_tool};
use qol_terminal_sessions::SessionId;

fn session(native: &str) -> SessionId {
    SessionId::new(qol_terminal_sessions::kitty::backend_id().clone(), native).unwrap()
}

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
    let claude = Notice::new(
        session("1"),
        &claude_tool(),
        "improve-logging".to_string(),
        "needs you",
    );
    assert_eq!(claude.title, "improve-logging");
    assert_eq!(claude.body, "Claude \u{00B7} needs you");

    let generic = Notice::new(
        session("2"),
        &generic_tool(),
        "qol dev".to_string(),
        "your turn",
    );
    assert_eq!(generic.body, "your turn", "generic carries no tool prefix");
}

#[test]
fn notice_click_focuses_its_own_session() {
    let notice = Notice::new(
        session("7"),
        &claude_tool(),
        "lane".to_string(),
        "needs you",
    );
    let activate = notice.activate();
    assert_eq!(activate.action, "focus");
    let target: SessionId = serde_json::from_value(activate.input["session"].clone()).unwrap();
    assert_eq!(target, session("7"));
}
