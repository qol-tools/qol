use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use plugin_cli_sessions::attention::{Evidence, GRACE_SECS};
use plugin_cli_sessions::daemon::reconcile::{
    tick, tick_with_caches, transition_line, ReconcileCaches,
};
use plugin_cli_sessions::host::{kitty_session_id, Pane, TerminalHost};
use plugin_cli_sessions::registry::{Registry, SessionState};
use plugin_cli_sessions::service::{NoServiceProbe, ServiceProbe};
use plugin_cli_sessions::status::Status;
use qol_terminal_sessions::cli::{
    codex_tool, generic_tool, kimi_tool, CliActivityEvidence, CliRuntimeState,
    CliSessionChangeHandler, CliSessionDescriptor, CliSessionEvidence, CliSessionInterpreter,
    CliSessionStrategy, CliSessionSubscription, CliTool, CliViewportState,
};
use qol_terminal_sessions::SessionBinding;

struct FakeHost {
    panes: Vec<Pane>,
    screen: String,
}

impl TerminalHost for FakeHost {
    fn discover(&self) -> Vec<Pane> {
        self.panes.clone()
    }
    fn get_text(&self, _target: &SessionBinding) -> Option<String> {
        Some(self.screen.clone())
    }
    fn focus(&self, _target: &SessionBinding) -> anyhow::Result<()> {
        Ok(())
    }
}

struct YesService;

impl ServiceProbe for YesService {
    fn is_service(&self, _pane: &Pane) -> bool {
        true
    }
}

struct CountingHost {
    panes: Vec<Pane>,
    screen: Mutex<String>,
    reads: AtomicUsize,
}

impl TerminalHost for CountingHost {
    fn discover(&self) -> Vec<Pane> {
        self.panes.clone()
    }

    fn get_text(&self, _target: &SessionBinding) -> Option<String> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Some(self.screen.lock().unwrap().clone())
    }

    fn focus(&self, _target: &SessionBinding) -> anyhow::Result<()> {
        Ok(())
    }
}

struct SubscribedAgent {
    tool: CliTool,
    process: String,
    handlers: Arc<Mutex<Vec<CliSessionChangeHandler>>>,
    external_id: Arc<Mutex<String>>,
    name: Arc<Mutex<Option<String>>>,
    runtime: Arc<Mutex<CliRuntimeState>>,
}

struct SubscribedHarness {
    interpreter: CliSessionInterpreter,
    handlers: Arc<Mutex<Vec<CliSessionChangeHandler>>>,
    external_id: Arc<Mutex<String>>,
    name: Arc<Mutex<Option<String>>>,
    runtime: Arc<Mutex<CliRuntimeState>>,
}

impl CliSessionStrategy for SubscribedAgent {
    fn tool(&self) -> &CliTool {
        &self.tool
    }

    fn matches(&self, pane: &Pane) -> bool {
        pane.foreground_basenames
            .iter()
            .any(|name| name == &self.process)
    }

    fn describe(&self, _pane: &Pane) -> CliSessionDescriptor {
        CliSessionDescriptor {
            tool: self.tool.clone(),
            display_name: self.name.lock().unwrap().clone(),
            external_id: Some(self.external_id.lock().unwrap().clone()),
            external_id_authoritative: false,
            has_activity: Some(true),
            evidence: CliSessionEvidence {
                runtime: *self.runtime.lock().unwrap(),
                activity: CliActivityEvidence::default(),
            },
        }
    }

    fn subscribe(
        &self,
        _pane: &Pane,
        on_change: CliSessionChangeHandler,
    ) -> Result<
        Option<CliSessionSubscription>,
        qol_terminal_sessions::cli::CliSessionSubscriptionError,
    > {
        self.handlers.lock().unwrap().push(on_change);
        Ok(Some(CliSessionSubscription::from_guard(())))
    }
}

struct FakeCodex {
    tool: CliTool,
    name: Option<String>,
    touched: bool,
}

impl CliSessionStrategy for FakeCodex {
    fn tool(&self) -> &CliTool {
        &self.tool
    }

    fn priority(&self) -> i32 {
        200
    }

    fn matches(&self, pane: &Pane) -> bool {
        pane.foreground_basenames.iter().any(|name| name == "codex")
    }

    fn describe(&self, _pane: &Pane) -> CliSessionDescriptor {
        CliSessionDescriptor {
            tool: self.tool.clone(),
            display_name: self.name.clone(),
            external_id: Some("test-session".to_owned()),
            external_id_authoritative: false,
            has_activity: Some(self.touched),
            evidence: CliSessionEvidence {
                runtime: if self.touched {
                    CliRuntimeState::Ready
                } else {
                    CliRuntimeState::Unknown
                },
                activity: CliActivityEvidence::default(),
            },
        }
    }
}

fn interpreter() -> CliSessionInterpreter {
    CliSessionInterpreter::system()
}

fn fake_codex(name: &str, touched: bool) -> CliSessionInterpreter {
    CliSessionInterpreter::from_strategies([Arc::new(FakeCodex {
        tool: codex_tool(),
        name: Some(name.to_owned()),
        touched,
    }) as Arc<dyn CliSessionStrategy>])
    .unwrap()
}

fn subscribed_agent(tool: CliTool, process: &str) -> SubscribedHarness {
    let handlers = Arc::new(Mutex::new(Vec::new()));
    let external_id = Arc::new(Mutex::new("subscribed-session".to_owned()));
    let name = Arc::new(Mutex::new(Some("Subscribed".to_owned())));
    let runtime = Arc::new(Mutex::new(CliRuntimeState::Working));
    let interpreter = CliSessionInterpreter::from_strategies([Arc::new(SubscribedAgent {
        tool,
        process: process.to_owned(),
        handlers: handlers.clone(),
        external_id: external_id.clone(),
        name: name.clone(),
        runtime: runtime.clone(),
    })
        as Arc<dyn CliSessionStrategy>])
    .unwrap();
    SubscribedHarness {
        interpreter,
        handlers,
        external_id,
        name,
        runtime,
    }
}

fn pane(window_id: u64, title: &str, at_prompt: bool, fg: &[&str], cmd: &str) -> Pane {
    Pane {
        id: kitty_session_id(window_id),
        root_pid: std::process::id() as i32,
        cwd: "/a/proj".into(),
        title: title.into(),
        at_prompt,
        reported_cmd: Some(cmd.into()),
        foreground_basenames: fg.iter().map(|s| s.to_string()).collect(),
        foreground_pids: vec![],
        capabilities: qol_terminal_sessions::SessionCapabilities::ALL,
        spawn_identity: None,
    }
}

fn restored(window_id: u64, status: Status) -> SessionState {
    SessionState {
        id: kitty_session_id(window_id),
        root_pid: std::process::id() as i32,
        project: "proj".into(),
        name: None,
        cwd: "/a/proj".into(),
        branch: None,
        tool: codex_tool(),
        status,
        summary: "x".into(),
        last_activity: 1,
        screen_hash: None,
        working_since: None,
        settled_since: None,
        bridged: false,
        driving: Vec::new(),
        runtime_status: None,
    }
}

const SELECTION: &str = "\u{276F} 1. Yes\n  2. No\n  enter to confirm";
const CODEX_DONE: &str = "";
const CLAUDE_WORKING: &str = "\u{273B} Processing\u{2026} (5s \u{00B7} \u{2193} 1k tokens)";
const CLAUDE_DONE: &str = "\u{273B} Brewed for 1m";
const KIMI_PICKER: &str = "? Choose a repair strategy\n\n\u{2192} [1] Repair now\n  [2] Defer\n\n\u{2191}\u{2193} select 1-2 / \u{21B5} choose \u{2190}/\u{2192} tab switch esc cancel";
const KIMI_TRANSITION: &str = "Collected your answers\nQ  Which repair?\n\u{2192} Repair now";
const KIMI_WORKING: &str =
    "Collected your answers\nQ  Which repair?\n\u{2192} Repair now\n\n\u{280B} thinking...\nyolo  K3-256k thinking: low  \u{2026}/qol-monorepo  main [\u{00B1}]\ncontext: 17% (41.1k/256k)";
const KIMI_EDITING_QUESTIONNAIRE: &str =
    include_str!("fixtures/kimi_real/questionnaire_editing.txt");
const KIMI_STALE_LINE: &str =
    "\u{280B} working... \u{00B7} Tip: ask Kimi to schedule tasks\n\n\u{256D}\u{2500}\u{2500}\u{2500}\u{256E}\n\u{2502} >  \u{2502}\n\u{2570}\u{2500}\u{2500}\u{2500}\u{256F}\nyolo  K3-256k thinking: low  \u{2026}/qol-monorepo  main [\u{00B1}]\ncontext: 17% (41.1k/256k)";
const CODEX_WORKING_LINE: &str = "  esc to interrupt ";
const CODEX_ANSWER_LIST: &str = "What remains:\n1. Add golden parity tests\n2. Decide when to remove trace-py\n3. Remove the fallback flag\n4. Rename the WIP commit";

#[test]
fn subscribed_screens_cache_after_active_sessions_settle() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let panes = (1..=6)
        .map(|id| pane(id, "qol-monorepo", false, &["zsh", "codex"], "codex"))
        .collect();
    let host = CountingHost {
        panes,
        screen: Mutex::new(CODEX_DONE.to_owned()),
        reads: AtomicUsize::new(0),
    };
    let harness = subscribed_agent(codex_tool(), "codex");
    let mut caches = ReconcileCaches::default();

    tick_with_caches(
        &reg,
        &host,
        &harness.interpreter,
        &NoServiceProbe,
        100,
        100,
        &mut caches,
    );
    assert_eq!(host.reads.load(Ordering::Relaxed), 6);

    *harness.runtime.lock().unwrap() = CliRuntimeState::Ready;
    tick_with_caches(
        &reg,
        &host,
        &harness.interpreter,
        &NoServiceProbe,
        110,
        110,
        &mut caches,
    );
    assert_eq!(
        host.reads.load(Ordering::Relaxed),
        12,
        "active subscribed panes get a confirming screen read while the turn settles"
    );
    for now in 111..=115 {
        tick_with_caches(
            &reg,
            &host,
            &harness.interpreter,
            &NoServiceProbe,
            now,
            now,
            &mut caches,
        );
    }
    assert_eq!(
        host.reads.load(Ordering::Relaxed),
        12 + 5 * 6,
        "settling panes stay active until the grace window closes"
    );
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::YourTurn,
        "the ready evidence completes the turn once the grace window closes"
    );

    harness.handlers.lock().unwrap()[2]();
    tick_with_caches(
        &reg,
        &host,
        &harness.interpreter,
        &NoServiceProbe,
        116,
        116,
        &mut caches,
    );
    assert_eq!(
        host.reads.load(Ordering::Relaxed),
        12 + 5 * 6 + 1,
        "a metadata signal refreshes only one calm pane"
    );

    tick_with_caches(
        &reg,
        &host,
        &harness.interpreter,
        &NoServiceProbe,
        176,
        176,
        &mut caches,
    );
    assert_eq!(
        host.reads.load(Ordering::Relaxed),
        12 + 5 * 6 + 1 + 6,
        "every subscribed pane still has a bounded full-screen fallback"
    );
}

#[test]
fn changed_external_session_replaces_the_metadata_subscription() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = CountingHost {
        panes: vec![pane(1, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: Mutex::new(SELECTION.to_owned()),
        reads: AtomicUsize::new(0),
    };
    let harness = subscribed_agent(codex_tool(), "codex");
    let mut caches = ReconcileCaches::default();

    tick_with_caches(
        &reg,
        &host,
        &harness.interpreter,
        &NoServiceProbe,
        100,
        100,
        &mut caches,
    );
    assert_eq!(harness.handlers.lock().unwrap().len(), 1);

    *harness.external_id.lock().unwrap() = "replacement-session".to_owned();
    tick_with_caches(
        &reg,
        &host,
        &harness.interpreter,
        &NoServiceProbe,
        101,
        101,
        &mut caches,
    );

    assert_eq!(harness.handlers.lock().unwrap().len(), 2);
    assert_eq!(host.reads.load(Ordering::Relaxed), 2);
}

#[test]
fn transient_missing_label_keeps_the_last_known_title() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let mut host = FakeHost {
        panes: vec![pane(2, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: SELECTION.to_owned(),
    };
    let harness = subscribed_agent(codex_tool(), "codex");
    let mut caches = ReconcileCaches::default();

    tick_with_caches(
        &reg,
        &host,
        &harness.interpreter,
        &NoServiceProbe,
        100,
        100,
        &mut caches,
    );
    assert_eq!(
        reg.lock().unwrap().sorted()[0].name.as_deref(),
        Some("Subscribed")
    );

    for (now, name, expected) in [
        (101, Some(" "), Some("Subscribed")),
        (102, Some("\u{1}"), Some("Subscribed")),
        (103, None, Some("Subscribed")),
        (104, Some("Renamed"), Some("Renamed")),
    ] {
        *harness.name.lock().unwrap() = name.map(str::to_owned);
        tick_with_caches(
            &reg,
            &host,
            &harness.interpreter,
            &NoServiceProbe,
            now,
            now,
            &mut caches,
        );
        assert_eq!(
            reg.lock().unwrap().sorted()[0].name.as_deref(),
            expected,
            "incoming label at {now:?} must not erase a stable title"
        );
    }

    *harness.name.lock().unwrap() = None;
    host.panes[0].root_pid = host.panes[0].root_pid.saturating_add(1);
    tick_with_caches(
        &reg,
        &host,
        &harness.interpreter,
        &NoServiceProbe,
        105,
        105,
        &mut caches,
    );
    assert_eq!(
        reg.lock().unwrap().sorted()[0].name,
        None,
        "a replaced pane identity must not inherit a stale title"
    );
}

#[test]
fn unsubscribed_screens_still_read_every_tick() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = CountingHost {
        panes: vec![pane(1, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: Mutex::new(SELECTION.to_owned()),
        reads: AtomicUsize::new(0),
    };
    let interpreter = fake_codex("Unsubscribed", true);
    let mut caches = ReconcileCaches::default();

    for now in [100, 101] {
        tick_with_caches(
            &reg,
            &host,
            &interpreter,
            &NoServiceProbe,
            now,
            now,
            &mut caches,
        );
    }

    assert_eq!(host.reads.load(Ordering::Relaxed), 2);
}

#[test]
fn answered_picker_stays_working_through_the_transition() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = CountingHost {
        panes: vec![pane(1, "project", false, &["zsh", "kimi"], "kimi")],
        screen: Mutex::new(KIMI_PICKER.to_owned()),
        reads: AtomicUsize::new(0),
    };
    let mut caches = ReconcileCaches::default();

    tick_with_caches(
        &reg,
        &host,
        &interpreter(),
        &NoServiceProbe,
        100,
        100,
        &mut caches,
    );
    tick_with_caches(
        &reg,
        &host,
        &interpreter(),
        &NoServiceProbe,
        101,
        101,
        &mut caches,
    );
    assert_eq!(reg.lock().unwrap().sorted()[0].status, Status::NeedsYou);

    *host.screen.lock().unwrap() = KIMI_TRANSITION.to_owned();
    tick_with_caches(
        &reg,
        &host,
        &interpreter(),
        &NoServiceProbe,
        102,
        102,
        &mut caches,
    );
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::NeedsYou,
        "a chrome-less transition frame holds the answered session instead of flipping it"
    );

    *host.screen.lock().unwrap() = KIMI_WORKING.to_owned();
    tick_with_caches(
        &reg,
        &host,
        &interpreter(),
        &NoServiceProbe,
        103,
        103,
        &mut caches,
    );

    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "the first spinner frame must keep the answered session working"
    );
    assert_eq!(
        host.reads.load(Ordering::Relaxed),
        4,
        "active transitions read fresh screen text without metadata signals"
    );
}

#[test]
fn kimi_questionnaire_enters_attention_after_confirmation_and_keeps_it_while_editing() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = CountingHost {
        panes: vec![pane(1, "project", false, &["zsh", "kimi"], "kimi")],
        screen: Mutex::new(KIMI_WORKING.to_owned()),
        reads: AtomicUsize::new(0),
    };
    let mut caches = ReconcileCaches::default();

    tick_with_caches(
        &reg,
        &host,
        &interpreter(),
        &NoServiceProbe,
        100,
        100,
        &mut caches,
    );
    assert_eq!(reg.lock().unwrap().sorted()[0].status, Status::Working);

    *host.screen.lock().unwrap() = KIMI_EDITING_QUESTIONNAIRE.to_owned();
    tick_with_caches(
        &reg,
        &host,
        &interpreter(),
        &NoServiceProbe,
        101,
        101,
        &mut caches,
    );
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "a questionnaire frame right after work with unknown freshness is not confirmed yet"
    );

    *host.screen.lock().unwrap() =
        KIMI_EDITING_QUESTIONNAIRE.replace("custom answer", "changed answer");
    tick_with_caches(
        &reg,
        &host,
        &interpreter(),
        &NoServiceProbe,
        102,
        102,
        &mut caches,
    );
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "an editing frame keeps the confirmation window open"
    );

    for now in 103..=107 {
        tick_with_caches(
            &reg,
            &host,
            &interpreter(),
            &NoServiceProbe,
            now,
            now,
            &mut caches,
        );
    }
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "a settled questionnaire still needs the full grace"
    );
    tick_with_caches(
        &reg,
        &host,
        &interpreter(),
        &NoServiceProbe,
        108,
        108,
        &mut caches,
    );
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::NeedsYou,
        "a settled questionnaire confirmed by the grace alerts"
    );

    *host.screen.lock().unwrap() =
        KIMI_EDITING_QUESTIONNAIRE.replace("changed answer", "another answer");
    tick_with_caches(
        &reg,
        &host,
        &interpreter(),
        &NoServiceProbe,
        109,
        109,
        &mut caches,
    );
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::NeedsYou,
        "editing after confirmation keeps attention"
    );
    assert_eq!(host.reads.load(Ordering::Relaxed), 10);
}

#[test]
fn kimi_stale_spinner_line_settles_to_your_turn_after_grace() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(1, "project", false, &["zsh", "kimi"], "kimi")],
        screen: KIMI_STALE_LINE.into(),
    };
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100, 100);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "a first sighting of a spinner line is still conservative"
    );
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 101, 101);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "one stable poll is insufficient"
    );
    for now in 102..=106 {
        tick(&reg, &host, &interpreter(), &NoServiceProbe, now, now);
    }
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::YourTurn,
        "a settled screen with a stale spinner line completes after the grace window"
    );
}

#[test]
fn kimi_scrolled_screen_holds_working_until_the_live_view_returns() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let mut host = FakeHost {
        panes: vec![pane(1, "project", false, &["zsh", "kimi"], "kimi")],
        screen: KIMI_WORKING.into(),
    };
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100, 100);
    assert_eq!(reg.lock().unwrap().sorted()[0].status, Status::Working);

    let scrolled = "User message number 57 asking about topic 57.\n\nAssistant reply number 57 with a fairly long answer\nthat wraps across multiple terminal lines.";
    host.screen = scrolled.to_owned();
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 101, 101);
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 102, 102);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "a scrolled chrome-less view must hold the working status, not flip to your turn"
    );
    assert_eq!(
        reg.lock().unwrap().sorted()[0].last_activity,
        100,
        "the stretch start survives the scrolled hold"
    );

    host.screen = KIMI_STALE_LINE.to_owned();
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 103, 103);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "the returning spinner frame reads live work"
    );
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 104, 104);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "one settled poll after the view returns is not yet your turn"
    );
    for now in 105..=109 {
        tick(&reg, &host, &interpreter(), &NoServiceProbe, now, now);
    }
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::YourTurn,
        "the settled stale line is your turn once the grace window closes"
    );
}

#[test]
fn tick_classifies_codex_blocked_from_screen() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(11, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: SELECTION.into(),
    };
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100, 100);
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 101, 101);
    let rows = reg.lock().unwrap().sorted();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].status,
        Status::NeedsYou,
        "a selection box on the screen => blocked => red"
    );
}

#[test]
fn tick_codex_idle_when_no_turn_taken() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let cli_interpreter = fake_codex("Asasdsadasd", false);
    let host = FakeHost {
        panes: vec![pane(12, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: String::new(),
    };
    tick(&reg, &host, &cli_interpreter, &NoServiceProbe, 100, 100);
    tick(&reg, &host, &cli_interpreter, &NoServiceProbe, 101, 101);
    let rows = reg.lock().unwrap().sorted();
    assert_eq!(
        rows[0].status,
        Status::Unknown,
        "no turn in the rollout => idle, not your turn"
    );
    assert_eq!(rows[0].name.as_deref(), Some("Asasdsadasd"),);
}

#[test]
fn tick_codex_ready_state_clears_a_stale_needs_you_status() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    reg.lock()
        .unwrap()
        .restore(vec![restored(13, Status::NeedsYou)]);
    let host = FakeHost {
        panes: vec![pane(
            13,
            "qol-monorepo | Action Required | Ready | gpt-5.6-luna max",
            false,
            &["zsh", "codex"],
            "codex",
        )],
        screen: String::new(),
    };

    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100, 100);
    assert_eq!(reg.lock().unwrap().sorted()[0].status, Status::NeedsYou);
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 101, 101);
    tick(
        &reg,
        &host,
        &interpreter(),
        &NoServiceProbe,
        101 + GRACE_SECS,
        101 + GRACE_SECS,
    );

    assert_eq!(reg.lock().unwrap().sorted()[0].status, Status::Unknown);
}

#[test]
fn tick_codex_requires_ready_evidence_even_when_answer_ends_in_numbered_list() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let mut host = FakeHost {
        panes: vec![pane(16, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: CODEX_WORKING_LINE.into(),
    };
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100, 100);
    assert_eq!(reg.lock().unwrap().sorted()[0].status, Status::Working);

    host.screen = CODEX_ANSWER_LIST.to_owned();
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 101, 101);
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 102, 102);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "a settled answer without live or fresh evidence holds until the grace window closes"
    );
    for now in 103..=107 {
        tick(&reg, &host, &interpreter(), &NoServiceProbe, now, now);
    }
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "a quiet answer is not proof of completion"
    );

    host.panes[0].title = "project | Ready | finished".into();
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 108, 108);
    assert_eq!(reg.lock().unwrap().sorted()[0].status, Status::YourTurn);
}

#[test]
fn tick_returns_attention_notice_for_new_needs_you() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(40, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: SELECTION.into(),
    };
    let notices = tick(&reg, &host, &interpreter(), &NoServiceProbe, 100, 100);
    assert_eq!(
        notices.len(),
        1,
        "strong needs-input emits one notice immediately"
    );
    assert_eq!(notices[0].body, "Codex \u{00B7} needs you");
}

#[test]
fn tick_does_not_repeat_notice_while_status_holds() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(41, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: SELECTION.into(),
    };
    let first = tick(&reg, &host, &interpreter(), &NoServiceProbe, 100, 100);
    assert_eq!(first.len(), 1, "the transition into needs-you notifies");
    let second = tick(&reg, &host, &interpreter(), &NoServiceProbe, 101, 101);
    assert!(second.is_empty(), "staying needs-you must not re-notify");
}

#[test]
fn tick_generic_listening_reads_service() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(30, "qol dev", false, &["zsh", "qol"], "qol dev")],
        screen: String::new(),
    };
    tick(&reg, &host, &interpreter(), &YesService, 100, 100);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Service,
        "a generic process holding a listener reads live, not working"
    );
}

#[test]
fn tick_agent_never_reads_service() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(
            31,
            "\u{2733} proj",
            false,
            &["zsh", "claude"],
            "claude",
        )],
        screen: "Welcome to Claude Code\n\u{276F} ".into(),
    };
    tick(&reg, &host, &interpreter(), &YesService, 100, 100);
    tick(&reg, &host, &interpreter(), &YesService, 101, 101);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Unknown,
        "an agent is never demoted to live even when the probe says yes"
    );
}

#[test]
fn tick_codex_your_turn_when_turn_taken() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    reg.lock()
        .unwrap()
        .restore(vec![restored(13, Status::Working)]);
    let cli_interpreter = fake_codex("Asasdsadasd", true);
    let host = FakeHost {
        panes: vec![pane(13, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: String::new(),
    };
    tick(&reg, &host, &cli_interpreter, &NoServiceProbe, 100, 100);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "the confirming screen read resets the settle stretch"
    );
    for now in 101..=106 {
        tick(&reg, &host, &cli_interpreter, &NoServiceProbe, now, now);
    }
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::YourTurn,
        "a completed rollout is your turn once the grace window closes"
    );
}

#[test]
fn tick_marks_claude_your_turn_then_keeps_ack() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let working = FakeHost {
        panes: vec![pane(
            10,
            "\u{2733} proj",
            false,
            &["zsh", "claude"],
            "claude",
        )],
        screen: CLAUDE_WORKING.into(),
    };
    tick(&reg, &working, &interpreter(), &NoServiceProbe, 100, 100);
    tick(&reg, &working, &interpreter(), &NoServiceProbe, 101, 101);
    assert_eq!(reg.lock().unwrap().sorted()[0].status, Status::Working);

    let parked = FakeHost {
        panes: vec![pane(
            10,
            "\u{2733} proj",
            false,
            &["zsh", "claude"],
            "claude",
        )],
        screen: CLAUDE_DONE.into(),
    };
    tick(&reg, &parked, &interpreter(), &NoServiceProbe, 200, 200);
    tick(&reg, &parked, &interpreter(), &NoServiceProbe, 201, 201);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "a done marker needs the settle grace before it completes"
    );
    for now in 202..=206 {
        tick(&reg, &parked, &interpreter(), &NoServiceProbe, now, now);
    }
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::YourTurn,
        "the parked done summary completes after the grace window"
    );

    reg.lock()
        .unwrap()
        .get_mut(&kitty_session_id(10))
        .unwrap()
        .acknowledge();
    tick(&reg, &parked, &interpreter(), &NoServiceProbe, 300, 300);
    assert_eq!(reg.lock().unwrap().sorted()[0].status, Status::Acknowledged,);

    let redrawn = FakeHost {
        panes: vec![pane(
            10,
            "\u{2733} proj",
            false,
            &["zsh", "claude"],
            "claude",
        )],
        screen: "\u{273B} Brewed for 1m\n(rename redraw line)".into(),
    };
    tick(&reg, &redrawn, &interpreter(), &NoServiceProbe, 400, 400);
    tick(&reg, &redrawn, &interpreter(), &NoServiceProbe, 401, 401);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Acknowledged,
        "a cosmetic redraw must not re-arm an acknowledged turn"
    );
}

#[test]
fn tick_claude_fresh_is_idle() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(
            14,
            "\u{2733} proj",
            false,
            &["zsh", "claude"],
            "claude",
        )],
        screen: "Welcome to Claude Code\n\u{276F} ".into(),
    };
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100, 100);
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 101, 101);
    assert_eq!(reg.lock().unwrap().sorted()[0].status, Status::Unknown,);
}

#[test]
fn tick_labels_claude_from_title_not_launch_alias() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(
            20,
            "\u{2733} Improve logging",
            false,
            &["zsh", "claude"],
            "claudedw",
        )],
        screen: String::new(),
    };
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100, 100);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].name.as_deref(),
        Some("Improve logging"),
        "claude label is the title topic, not the launch alias"
    );
}

#[test]
fn tick_labels_generic_from_command() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(21, "qol dev", false, &["zsh", "qol"], "qol dev")],
        screen: String::new(),
    };
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100, 100);
    let rows = reg.lock().unwrap().sorted();
    assert_eq!(rows[0].name.as_deref(), Some("qol dev"));
    assert_eq!(rows[0].status, Status::Working, "running generic => green");
}

#[test]
fn tick_does_not_refresh_activity_timestamp_while_working() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(22, "qol dev", false, &["zsh", "qol"], "qol dev")],
        screen: String::new(),
    };

    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100, 100);
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 200, 200);

    let rows = reg.lock().unwrap().sorted();
    assert_eq!(rows[0].last_activity, 100);
}

#[test]
fn tick_refreshes_restored_identity_fields() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    reg.lock().unwrap().restore(vec![SessionState {
        id: kitty_session_id(21),
        root_pid: -1,
        project: "stale".into(),
        name: None,
        cwd: "/stale".into(),
        branch: None,
        tool: generic_tool(),
        status: Status::Unknown,
        summary: "idle".into(),
        last_activity: 1,
        screen_hash: None,
        working_since: None,
        settled_since: None,
        bridged: false,
        driving: Vec::new(),
        runtime_status: None,
    }]);
    let host = FakeHost {
        panes: vec![pane(21, "qol dev", false, &["zsh", "qol"], "qol dev")],
        screen: String::new(),
    };

    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100, 100);

    let rows = reg.lock().unwrap().sorted();
    assert_eq!(rows[0].project, "proj");
    assert_eq!(rows[0].cwd, "/a/proj");
    assert_ne!(rows[0].root_pid, -1);
}

#[test]
fn tick_drops_panes_that_disappear() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let present = FakeHost {
        panes: vec![pane(
            10,
            "\u{2733} proj",
            false,
            &["zsh", "claude"],
            "claude",
        )],
        screen: String::new(),
    };
    tick(&reg, &present, &interpreter(), &NoServiceProbe, 100, 100);
    assert_eq!(reg.lock().unwrap().sorted().len(), 1);

    let gone = FakeHost {
        panes: vec![],
        screen: String::new(),
    };
    tick(&reg, &gone, &interpreter(), &NoServiceProbe, 200, 200);
    assert_eq!(
        reg.lock().unwrap().sorted().len(),
        0,
        "a pane that disappears is pruned"
    );
}

#[test]
fn restored_attention_never_re_alerts_without_evidence() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    reg.lock()
        .unwrap()
        .restore(vec![restored(50, Status::Acknowledged)]);
    let host = FakeHost {
        panes: vec![pane(50, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: CODEX_DONE.into(),
    };
    let notices = tick(&reg, &host, &interpreter(), &NoServiceProbe, 100, 100);
    assert!(
        notices.is_empty(),
        "a restored acknowledged session must not re-announce"
    );
    assert_eq!(reg.lock().unwrap().sorted()[0].status, Status::Acknowledged);
}

#[test]
fn transition_diagnostics_are_redacted_and_carry_the_reason() {
    let evidence = Evidence {
        descriptor_runtime: CliRuntimeState::Unknown,
        screen_runtime: CliRuntimeState::Ready,
        viewport: CliViewportState::Unknown,
        file_fresh: None,
        file_quiet_secs: None,
        screen_changed: false,
        at_prompt: false,
        is_generic: false,
        is_service: false,
    };
    let line = transition_line(
        "kitty:7",
        "claude",
        Status::Working,
        Status::YourTurn,
        plugin_cli_sessions::attention::Reason::GraceCompleted,
        5,
        &evidence,
    );
    assert!(
        line.contains("id=kitty:7"),
        "session identity is bounded: {line}"
    );
    assert!(line.contains("tool=claude"), "tool is named: {line}");
    assert!(
        line.contains("prev=Working"),
        "prev status is named: {line}"
    );
    assert!(line.contains("new=YourTurn"), "new status is named: {line}");
    assert!(
        line.contains("reason=GraceCompleted"),
        "reason is named: {line}"
    );
    assert!(
        line.contains("grace_s=5"),
        "grace elapsed is bounded: {line}"
    );
    assert!(
        !line.contains("✻") && !line.contains("Brewed"),
        "no screen content may leak into the diagnostic: {line}"
    );
    assert_eq!(
        line.len(),
        line.trim().len(),
        "the diagnostic is a single line"
    );
}

#[test]
fn completing_after_grace_requires_a_time_advance_not_poll_count() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(60, "project", false, &["zsh", "kimi"], "kimi")],
        screen: KIMI_STALE_LINE.into(),
    };
    for _ in 0..20 {
        tick(&reg, &host, &interpreter(), &NoServiceProbe, 100, 100);
    }
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "twenty polls at the same instant are not a grace period"
    );
    tick(
        &reg,
        &host,
        &interpreter(),
        &NoServiceProbe,
        100 + GRACE_SECS,
        100 + GRACE_SECS,
    );
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::YourTurn,
        "one poll after the grace boundary completes the turn"
    );
}

#[test]
fn wall_clock_jumps_do_not_expire_grace() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let busy = FakeHost {
        panes: vec![pane(22, "qol dev", false, &["zsh", "qol"], "qol dev")],
        screen: String::new(),
    };
    let prompt = FakeHost {
        panes: vec![pane(22, "qol dev", true, &["zsh", "qol"], "qol dev")],
        screen: String::new(),
    };
    tick(&reg, &busy, &interpreter(), &NoServiceProbe, 100, 100);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "a generic busy pane starts working"
    );
    tick(&reg, &busy, &interpreter(), &NoServiceProbe, 1100, 101);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "a +1000s wall jump does not disturb a running turn"
    );
    tick(&reg, &prompt, &interpreter(), &NoServiceProbe, 1100, 104);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Unknown,
        "the wall jump must not expire the 5s grace early"
    );

    tick(&reg, &busy, &interpreter(), &NoServiceProbe, 1100, 106);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Working,
        "a new working episode starts"
    );
    tick(&reg, &prompt, &interpreter(), &NoServiceProbe, 1100, 111);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::YourTurn,
        "grace counts monotonic seconds, not wall seconds"
    );
}

#[test]
fn restored_working_with_same_screen_hash_starts_a_fresh_grace() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(1, "project", false, &["zsh", "kimi"], "kimi")],
        screen: KIMI_STALE_LINE.into(),
    };
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100, 100);
    assert_eq!(reg.lock().unwrap().sorted()[0].status, Status::Working);
    let hash = reg.lock().unwrap().sorted()[0].screen_hash;
    assert!(hash.is_some(), "the working turn produced a screen hash");

    let restarted = Arc::new(Mutex::new(Registry::default()));
    restarted.lock().unwrap().restore(vec![SessionState {
        id: kitty_session_id(1),
        root_pid: std::process::id() as i32,
        project: "proj".into(),
        name: None,
        cwd: "/a/proj".into(),
        branch: None,
        tool: kimi_tool(),
        status: Status::Working,
        summary: "working".into(),
        last_activity: 100,
        screen_hash: hash,
        working_since: None,
        settled_since: None,
        bridged: false,
        driving: Vec::new(),
        runtime_status: None,
    }]);
    let mut caches = ReconcileCaches::default();

    tick_with_caches(
        &restarted,
        &host,
        &interpreter(),
        &NoServiceProbe,
        100,
        100,
        &mut caches,
    );
    assert_eq!(
        restarted.lock().unwrap().sorted()[0].status,
        Status::Working,
        "a restored working turn with the same screen hash must not complete instantly"
    );
    tick_with_caches(
        &restarted,
        &host,
        &interpreter(),
        &NoServiceProbe,
        100,
        104,
        &mut caches,
    );
    assert_eq!(
        restarted.lock().unwrap().sorted()[0].status,
        Status::Working,
        "the fresh grace is still running"
    );
    tick_with_caches(
        &restarted,
        &host,
        &interpreter(),
        &NoServiceProbe,
        100,
        105,
        &mut caches,
    );
    assert_eq!(
        restarted.lock().unwrap().sorted()[0].status,
        Status::YourTurn,
        "the restart observes a full fresh grace before completing"
    );
}

#[test]
fn pi_embedded_working_recovers_and_stays_busy_across_quiet_ticks() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(1, "sl-skill", false, &["pi"], "pi")],
        screen: include_str!("fixtures/corpus/pi_embedded_working.txt").into(),
    };
    let mut caches = ReconcileCaches::default();
    let interpreter = CliSessionInterpreter::system();
    for now in [100, 101, 106, 160, 700] {
        tick_with_caches(
            &reg,
            &host,
            &interpreter,
            &NoServiceProbe,
            now,
            now,
            &mut caches,
        );
        assert_eq!(
            reg.lock().unwrap().sorted()[0].status,
            Status::Working,
            "time={now}"
        );
    }
}
