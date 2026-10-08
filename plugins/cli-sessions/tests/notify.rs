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

#[test]
fn a_finished_park_notice_falls_back_to_the_project_and_caps_the_report() {
    let record = qol_terminal_sessions::park::ParkRecord {
        id: "park-1-2".to_owned(),
        tool: "claude".to_owned(),
        cwd: "/git/qol-monorepo".to_owned(),
        external_id: "abc".to_owned(),
        model: None,
        effort: None,
        title: None,
        permission_mode: None,
        session: String::new(),
        command: vec![],
        created_at: 0,
        state: qol_terminal_sessions::park::ParkState::Finished,
        runner_pid: None,
        runner_identity: None,
        caller_closed: true,
        exit_code: Some(0),
        resumed_session: None,
        detail: None,
        report: Some("\u{00E9}".repeat(500)),
        runner_exe: None,
        notified: false,
    };
    let notice = Notice::finished_park(&record);
    assert_eq!(notice.title, "qol-monorepo");
    assert_eq!(notice.body.chars().count(), 401);
    assert!(notice.body.ends_with('\u{2026}'));
    assert_eq!(notice.activate().input["park"], "park-1-2");
}
