use plugin_cli_sessions::host::kitty_session_id;
use plugin_cli_sessions::registry::{meaningful_name, summary_for, Registry, SessionState};
use plugin_cli_sessions::status::Status;
use qol_terminal_sessions::cli::{claude_tool, codex_tool, generic_tool, kimi_tool, pi_tool};
use qol_terminal_sessions::SessionId;

#[test]
fn working_summary_is_tool_aware() {
    let cases = [
        (&generic_tool(), "running"),
        (&claude_tool(), "working"),
        (&codex_tool(), "working"),
        (&kimi_tool(), "working"),
        (&pi_tool(), "working"),
    ];
    for (tool, expected) in cases {
        assert_eq!(
            summary_for(Status::Working, tool),
            expected,
            "tool: {tool:?}"
        );
    }
    assert_eq!(summary_for(Status::YourTurn, &generic_tool()), "your turn");
}

#[test]
fn meaningful_name_rejects_terminal_controls_and_trims_outer_space() {
    assert_eq!(meaningful_name(Some("  project  ")), Some("project"));
    assert_eq!(meaningful_name(Some("\u{1}")), None);
    assert_eq!(meaningful_name(Some("project\u{1}")), None);
    assert_eq!(meaningful_name(Some(" \t ")), None);
}

fn state(window_id: u64, status: Status, last: u64) -> SessionState {
    SessionState {
        id: kitty_session_id(window_id),
        root_pid: window_id as i32,
        project: "proj".into(),
        name: None,
        cwd: "/a/b/proj".into(),
        branch: None,
        tool: generic_tool(),
        status,
        summary: "x".into(),
        last_activity: last,
        screen_hash: None,
        working_since: None,
        settled_since: None,
        bridged: false,
        driving: Vec::new(),
        runtime_status: None,
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
fn bridge_membership_does_not_override_displayed_state_priority() {
    let mut r = Registry::default();
    r.upsert(state(10, Status::Working, 1));
    let mut bridged_idle = state(11, Status::Unknown, 1);
    bridged_idle.bridged = true;
    r.upsert(bridged_idle);
    let mut bridged_working = state(12, Status::Working, 1);
    bridged_working.bridged = true;
    r.upsert(bridged_working);
    r.upsert(state(13, Status::YourTurn, 2));
    r.upsert(state(14, Status::Unknown, 1));
    r.upsert(state(15, Status::Acknowledged, 1));
    r.upsert(state(16, Status::Service, 1));
    let ids: Vec<_> = r
        .sorted()
        .into_iter()
        .map(|s| s.id.native().parse::<u64>().unwrap())
        .collect();
    assert_eq!(ids, vec![13, 10, 12, 16, 11, 14, 15]);
}

#[test]
fn agent_working_sorts_above_generic_running() {
    let mut r = Registry::default();
    r.upsert(state(10, Status::Working, 1));
    let mut agent = state(11, Status::Working, 1);
    agent.tool = claude_tool();
    r.upsert(agent);
    let ids: Vec<_> = r
        .sorted()
        .into_iter()
        .map(|s| s.id.native().parse::<u64>().unwrap())
        .collect();
    assert_eq!(ids, vec![11, 10]);
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

#[test]
fn every_state_reorders_live_without_changing_selected_session() {
    use plugin_cli_sessions::selection::Selection;

    let expected = [
        Status::NeedsYou,
        Status::YourTurn,
        Status::AwaitingReview,
        Status::Coordinating,
        Status::Working,
        Status::Service,
        Status::Unknown,
        Status::Acknowledged,
    ];
    let mut registry = Registry::default();
    for (index, status) in expected.into_iter().enumerate().rev() {
        registry.upsert(state(index as u64, status, 1));
    }
    let rows = registry.sorted();
    assert_eq!(
        rows.iter().map(|row| row.status).collect::<Vec<_>>(),
        expected
    );
    let mut selection = Selection::default();
    selection.select(kitty_session_id(4));
    registry.upsert(state(7, Status::NeedsYou, 2));
    registry.upsert(state(0, Status::Acknowledged, 2));
    let order: Vec<_> = registry.sorted().into_iter().map(|row| row.id).collect();
    assert_eq!(order.first(), Some(&kitty_session_id(7)));
    assert_eq!(order.last(), Some(&kitty_session_id(0)));
    assert_eq!(selection.resolved(&order), Some(kitty_session_id(4)));
    assert_eq!(selection.highlight_index(&order), Some(4));
}
