use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use plugin_cli_sessions::daemon::reconcile::{tick, tick_with_caches, ReconcileCaches};
use plugin_cli_sessions::host::{kitty_session_id, Pane, TerminalHost};
use plugin_cli_sessions::registry::{Registry, SessionState};
use plugin_cli_sessions::service::{NoServiceProbe, ServiceProbe};
use plugin_cli_sessions::status::Status;
use plugin_cli_sessions::tool::Tool;
use qol_terminal_sessions::cli::{
    codex_tool, CliSessionChangeHandler, CliSessionDescriptor, CliSessionInterpreter,
    CliSessionStrategy, CliSessionSubscription, CliTool,
};

struct FakeHost {
    panes: Vec<Pane>,
    screen: String,
}

impl TerminalHost for FakeHost {
    fn discover(&self) -> Vec<Pane> {
        self.panes.clone()
    }
    fn get_text(&self, _window_id: u64, _root_pid: i32) -> Option<String> {
        Some(self.screen.clone())
    }
    fn focus(&self, _window_id: u64, _root_pid: i32) -> anyhow::Result<()> {
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
    reads: AtomicUsize,
}

impl TerminalHost for CountingHost {
    fn discover(&self) -> Vec<Pane> {
        self.panes.clone()
    }

    fn get_text(&self, _window_id: u64, _root_pid: i32) -> Option<String> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Some(SELECTION.to_owned())
    }

    fn focus(&self, _window_id: u64, _root_pid: i32) -> anyhow::Result<()> {
        Ok(())
    }
}

struct SubscribedCodex {
    tool: CliTool,
    handlers: Arc<Mutex<Vec<CliSessionChangeHandler>>>,
    external_id: Arc<Mutex<String>>,
}

struct SubscribedHarness {
    interpreter: CliSessionInterpreter,
    handlers: Arc<Mutex<Vec<CliSessionChangeHandler>>>,
    external_id: Arc<Mutex<String>>,
}

impl CliSessionStrategy for SubscribedCodex {
    fn tool(&self) -> &CliTool {
        &self.tool
    }

    fn matches(&self, pane: &Pane) -> bool {
        pane.foreground_basenames.iter().any(|name| name == "codex")
    }

    fn describe(&self, _pane: &Pane) -> CliSessionDescriptor {
        CliSessionDescriptor {
            tool: self.tool.clone(),
            display_name: Some("Subscribed".to_owned()),
            external_id: Some(self.external_id.lock().unwrap().clone()),
            has_activity: Some(true),
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
            has_activity: Some(self.touched),
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

fn subscribed_codex() -> SubscribedHarness {
    let handlers = Arc::new(Mutex::new(Vec::new()));
    let external_id = Arc::new(Mutex::new("subscribed-session".to_owned()));
    let interpreter = CliSessionInterpreter::from_strategies([Arc::new(SubscribedCodex {
        tool: codex_tool(),
        handlers: handlers.clone(),
        external_id: external_id.clone(),
    })
        as Arc<dyn CliSessionStrategy>])
    .unwrap();
    SubscribedHarness {
        interpreter,
        handlers,
        external_id,
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
    }
}

const SELECTION: &str = "\u{276F} 1. Yes\n  2. No\n  enter to confirm";
const CLAUDE_PICKER: &str = "How should the uninstaller be invoked?\n\n1. gpui popup picker\n2. Keep terminal CLI\n3. Both: popup + CLI\n\nEnter to select \u{00B7} \u{2191}/\u{2193} to navigate \u{00B7} n to add notes \u{00B7} Esc to cancel";
const CLAUDE_WORKING: &str = "\u{273B} Processing\u{2026} (5s \u{00B7} \u{2193} 1k tokens)";
const CLAUDE_DONE: &str = "\u{273B} Brewed for 1m";

#[test]
fn subscribed_screens_use_signals_with_a_bounded_fallback() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let panes = (1..=6)
        .map(|id| pane(id, "qol-monorepo", false, &["zsh", "codex"], "codex"))
        .collect();
    let host = CountingHost {
        panes,
        reads: AtomicUsize::new(0),
    };
    let harness = subscribed_codex();
    let mut caches = ReconcileCaches::default();

    tick_with_caches(
        &reg,
        &host,
        &harness.interpreter,
        &NoServiceProbe,
        100,
        &mut caches,
    );
    assert_eq!(host.reads.load(Ordering::Relaxed), 6);

    tick_with_caches(
        &reg,
        &host,
        &harness.interpreter,
        &NoServiceProbe,
        110,
        &mut caches,
    );
    assert_eq!(
        host.reads.load(Ordering::Relaxed),
        6,
        "unchanged subscribed panes reuse their cached screen"
    );

    harness.handlers.lock().unwrap()[2]();
    tick_with_caches(
        &reg,
        &host,
        &harness.interpreter,
        &NoServiceProbe,
        111,
        &mut caches,
    );
    assert_eq!(
        host.reads.load(Ordering::Relaxed),
        7,
        "a metadata signal refreshes only its pane"
    );

    tick_with_caches(
        &reg,
        &host,
        &harness.interpreter,
        &NoServiceProbe,
        171,
        &mut caches,
    );
    assert_eq!(
        host.reads.load(Ordering::Relaxed),
        13,
        "every subscribed pane still has a bounded full-screen fallback"
    );
}

#[test]
fn changed_external_session_replaces_the_metadata_subscription() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = CountingHost {
        panes: vec![pane(1, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        reads: AtomicUsize::new(0),
    };
    let harness = subscribed_codex();
    let mut caches = ReconcileCaches::default();

    tick_with_caches(
        &reg,
        &host,
        &harness.interpreter,
        &NoServiceProbe,
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
        &mut caches,
    );

    assert_eq!(harness.handlers.lock().unwrap().len(), 2);
    assert_eq!(host.reads.load(Ordering::Relaxed), 2);
}

#[test]
fn unsubscribed_screens_still_read_every_tick() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = CountingHost {
        panes: vec![pane(1, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        reads: AtomicUsize::new(0),
    };
    let interpreter = fake_codex("Unsubscribed", true);
    let mut caches = ReconcileCaches::default();

    for now in [100, 101] {
        tick_with_caches(&reg, &host, &interpreter, &NoServiceProbe, now, &mut caches);
    }

    assert_eq!(host.reads.load(Ordering::Relaxed), 2);
}

#[test]
fn tick_classifies_codex_blocked_from_screen() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(11, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: SELECTION.into(),
    };
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100);
    let rows = reg.lock().unwrap().sorted();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].status,
        Status::NeedsYou,
        "a selection box on screen => blocked => red"
    );
}

#[test]
fn tick_claude_blocked_when_choice_picker_on_screen() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(
            15,
            "\u{2733} uninstaller",
            false,
            &["zsh", "claude"],
            "claude",
        )],
        screen: CLAUDE_PICKER.into(),
    };
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::NeedsYou,
        "a claude session showing a choice picker must read needs-you, not idle"
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
    tick(&reg, &host, &cli_interpreter, &NoServiceProbe, 100);
    let rows = reg.lock().unwrap().sorted();
    assert_eq!(
        rows[0].status,
        Status::Unknown,
        "no turn in the rollout => idle, not your turn"
    );
    assert_eq!(
        rows[0].name.as_deref(),
        Some("Asasdsadasd"),
        "label comes from the codex session store"
    );
}

#[test]
fn tick_codex_your_turn_when_answer_ends_in_numbered_list() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(16, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: "What remains:\n1. Add golden parity tests\n2. Decide when to remove trace-py\n3. Remove the fallback flag\n4. Rename the WIP commit".into(),
    };
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::YourTurn,
        "a codex answer ending in a numbered list is your-turn, not needs-you"
    );
}

#[test]
fn tick_returns_attention_notice_for_new_needs_you() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(40, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: SELECTION.into(),
    };
    let notices = tick(&reg, &host, &interpreter(), &NoServiceProbe, 100);
    assert_eq!(
        notices.len(),
        1,
        "a fresh session that needs you emits one notice"
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
    let first = tick(&reg, &host, &interpreter(), &NoServiceProbe, 100);
    assert_eq!(first.len(), 1, "first transition into needs-you notifies");
    let second = tick(&reg, &host, &interpreter(), &NoServiceProbe, 101);
    assert!(second.is_empty(), "staying needs-you must not re-notify");
}

#[test]
fn tick_generic_listening_reads_service() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(30, "qol dev", false, &["zsh", "qol"], "qol dev")],
        screen: String::new(),
    };
    tick(&reg, &host, &interpreter(), &YesService, 100);
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
    tick(&reg, &host, &interpreter(), &YesService, 100);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Unknown,
        "an agent is never demoted to live even when the probe says yes"
    );
}

#[test]
fn tick_codex_your_turn_when_turn_taken() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let cli_interpreter = fake_codex("Asasdsadasd", true);
    let host = FakeHost {
        panes: vec![pane(13, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: String::new(),
    };
    tick(&reg, &host, &cli_interpreter, &NoServiceProbe, 100);
    assert_eq!(reg.lock().unwrap().sorted()[0].status, Status::YourTurn);
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
    tick(&reg, &working, &interpreter(), &NoServiceProbe, 100);
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
    tick(&reg, &parked, &interpreter(), &NoServiceProbe, 200);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::YourTurn,
        "a completion marker means your turn"
    );

    reg.lock().unwrap().get_mut(10).unwrap().acknowledge();
    tick(&reg, &parked, &interpreter(), &NoServiceProbe, 300);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Acknowledged,
        "ack survives a tick while still parked"
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
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::Unknown,
        "no completion marker yet => idle"
    );
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
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100);
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
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100);
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

    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100);
    tick(&reg, &host, &interpreter(), &NoServiceProbe, 200);

    let rows = reg.lock().unwrap().sorted();
    assert_eq!(rows[0].last_activity, 100);
    assert_eq!(rows[0].running_since, Some(100));
}

#[test]
fn tick_refreshes_restored_identity_fields() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    reg.lock().unwrap().restore(vec![SessionState {
        window_id: 21,
        root_pid: -1,
        project: "stale".into(),
        name: None,
        cwd: "/stale".into(),
        branch: None,
        tool: Tool::Generic,
        status: Status::Unknown,
        summary: "idle".into(),
        last_activity: 1,
        screen_hash: None,
        running_since: None,
    }]);
    let host = FakeHost {
        panes: vec![pane(21, "qol dev", false, &["zsh", "qol"], "qol dev")],
        screen: String::new(),
    };

    tick(&reg, &host, &interpreter(), &NoServiceProbe, 100);

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
    tick(&reg, &present, &interpreter(), &NoServiceProbe, 100);
    assert_eq!(reg.lock().unwrap().sorted().len(), 1);

    let gone = FakeHost {
        panes: vec![],
        screen: String::new(),
    };
    tick(&reg, &gone, &interpreter(), &NoServiceProbe, 200);
    assert_eq!(
        reg.lock().unwrap().sorted().len(),
        0,
        "a window no longer present is removed"
    );
}
