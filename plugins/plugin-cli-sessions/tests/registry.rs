use plugin_cli_sessions::registry::{summary_for, Registry, SessionState};
use plugin_cli_sessions::status::Status;
use plugin_cli_sessions::tool::Tool;

#[test]
fn working_summary_is_tool_aware() {
    let cases = [
        (Tool::Generic, "running"),
        (Tool::Claude, "working"),
        (Tool::Codex, "working"),
    ];
    for (tool, expected) in cases {
        assert_eq!(
            summary_for(Status::Working, tool),
            expected,
            "tool: {tool:?}"
        );
    }
    assert_eq!(summary_for(Status::YourTurn, Tool::Generic), "your turn");
}

fn state(window_id: u64, status: Status, last: u64) -> SessionState {
    SessionState {
        window_id,
        root_pid: window_id as i32,
        project: "proj".into(),
        name: None,
        cwd: "/a/b/proj".into(),
        branch: None,
        tool: Tool::Generic,
        status,
        summary: "x".into(),
        last_activity: last,
        screen_hash: None,
        running_since: None,
    }
}

#[test]
fn upsert_is_last_writer_wins_by_window() {
    let mut r = Registry::default();
    r.upsert(state(1, Status::Working, 1));
    r.upsert(state(1, Status::NeedsYou, 2));
    let all = r.sorted();
    assert_eq!(all.len(), 1, "same window merges");
    assert_eq!(all[0].status, Status::NeedsYou);
}

#[test]
fn sorted_orders_red_yellow_green_unknown_ack() {
    let mut r = Registry::default();
    r.upsert(state(10, Status::Working, 1));
    r.upsert(state(12, Status::YourTurn, 2));
    r.upsert(state(13, Status::NeedsYou, 1));
    r.upsert(state(14, Status::Unknown, 9));
    let ids: Vec<_> = r.sorted().into_iter().map(|s| s.window_id).collect();
    assert_eq!(ids, vec![13, 12, 10, 14]);
}

#[test]
fn within_a_tier_more_recent_sorts_first() {
    let mut r = Registry::default();
    r.upsert(state(1, Status::NeedsYou, 5));
    r.upsert(state(2, Status::NeedsYou, 9));
    r.upsert(state(3, Status::YourTurn, 20));
    let ids: Vec<_> = r.sorted().into_iter().map(|s| s.window_id).collect();
    assert_eq!(
        ids,
        vec![2, 1, 3],
        "needs-you before your-turn; within needs-you the most recent comes first"
    );
}

#[test]
fn acknowledge_clears_your_turn_only() {
    let mut s = state(1, Status::YourTurn, 1);
    s.acknowledge();
    assert_eq!(s.status, Status::Acknowledged);
    assert_eq!(s.summary, "acknowledged");

    let mut w = state(1, Status::Working, 1);
    w.acknowledge();
    assert_eq!(w.status, Status::Working, "ack is a no-op off your-turn");
}
