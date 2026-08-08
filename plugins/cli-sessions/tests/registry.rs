use plugin_cli_sessions::host::kitty_session_id;
use plugin_cli_sessions::registry::{summary_for, Registry, SessionState};
use plugin_cli_sessions::status::Status;
use plugin_cli_sessions::tool::Tool;
use qol_terminal_sessions::SessionId;

#[test]
fn working_summary_is_tool_aware() {
    let cases = [
        (Tool::Generic, "running"),
        (Tool::Claude, "working"),
        (Tool::Codex, "working"),
        (Tool::Kimi, "working"),
        (Tool::Pi, "working"),
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
        id: kitty_session_id(window_id),
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
        working_since: None,
        settled_since: None,
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
fn equal_window_numbers_from_distinct_instances_coexist() {
    let mut first = state(3, Status::Working, 1);
    first.id =
        SessionId::new(qol_terminal_sessions::kitty::backend_id().clone(), "k1_1.3").unwrap();
    let mut second = state(3, Status::NeedsYou, 2);
    second.id =
        SessionId::new(qol_terminal_sessions::kitty::backend_id().clone(), "k1_2.3").unwrap();
    let mut registry = Registry::default();

    registry.upsert(first);
    registry.upsert(second);

    let ids = registry
        .sorted()
        .into_iter()
        .map(|state| state.id.native().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["k1_2.3", "k1_1.3"]);
}

#[test]
fn sorted_orders_red_yellow_green_unknown_ack() {
    let mut r = Registry::default();
    r.upsert(state(10, Status::Working, 1));
    r.upsert(state(12, Status::YourTurn, 2));
    r.upsert(state(13, Status::NeedsYou, 1));
    r.upsert(state(14, Status::Unknown, 9));
    let ids: Vec<_> = r
        .sorted()
        .into_iter()
        .map(|s| s.id.native().parse::<u64>().unwrap())
        .collect();
    assert_eq!(ids, vec![13, 12, 10, 14]);
}

#[test]
fn within_a_tier_orders_by_session_id_not_recency() {
    let mut r = Registry::default();
    r.upsert(state(2, Status::NeedsYou, 9));
    r.upsert(state(1, Status::NeedsYou, 5));
    r.upsert(state(3, Status::YourTurn, 20));
    let ids: Vec<_> = r
        .sorted()
        .into_iter()
        .map(|s| s.id.native().parse::<u64>().unwrap())
        .collect();
    assert_eq!(
        ids,
        vec![1, 2, 3],
        "needs-you before your-turn; within a tier, stable by window_id (not last_activity)"
    );
}

#[test]
fn same_tier_order_is_stable_when_activity_changes() {
    let order = |a_last: u64, b_last: u64| {
        let mut r = Registry::default();
        r.upsert(state(7, Status::YourTurn, a_last));
        r.upsert(state(4, Status::YourTurn, b_last));
        r.sorted()
            .into_iter()
            .map(|s| s.id.native().parse::<u64>().unwrap())
            .collect::<Vec<_>>()
    };
    let cases = [(1, 100), (100, 1), (50, 50)];
    for (a, b) in cases {
        assert_eq!(
            order(a, b),
            vec![4, 7],
            "rows must not reshuffle as last_activity ticks (a={a}, b={b})"
        );
    }
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

#[test]
fn settle_timers_are_never_serialized() {
    let mut s = state(1, Status::Working, 100);
    s.working_since = Some(42);
    s.settled_since = Some(50);
    let json = serde_json::to_string(&s).unwrap();
    assert!(
        !json.contains("working_since") && !json.contains("settled_since"),
        "transition timers must stay process-local: {json}"
    );
    let restored: SessionState = serde_json::from_str(&json).unwrap();
    assert_eq!(
        restored.working_since, None,
        "restore starts with no timers"
    );
    assert_eq!(
        restored.settled_since, None,
        "restore starts with no timers"
    );
    assert_eq!(restored.status, Status::Working, "status survives");
    assert_eq!(
        restored.last_activity, 100,
        "the wall display timestamp survives"
    );
}
