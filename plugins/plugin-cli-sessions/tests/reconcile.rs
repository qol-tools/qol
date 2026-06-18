use std::sync::{Arc, Mutex};

use plugin_cli_sessions::daemon::reconcile::tick;
use plugin_cli_sessions::host::{Pane, TerminalHost};
use plugin_cli_sessions::registry::Registry;
use plugin_cli_sessions::status::Status;
use plugin_cli_sessions::strategy::codex::{CodexSession, CodexStore, NoCodexStore};

struct FakeHost {
    panes: Vec<Pane>,
    screen: String,
}

impl TerminalHost for FakeHost {
    fn discover(&self) -> Vec<Pane> {
        self.panes.clone()
    }
    fn get_text(&self, _window_id: u64) -> Option<String> {
        Some(self.screen.clone())
    }
    fn focus(&self, _window_id: u64) -> anyhow::Result<()> {
        Ok(())
    }
}

struct FakeCodex {
    name: Option<String>,
    touched: bool,
}

impl CodexStore for FakeCodex {
    fn session(&self, _pane: &Pane) -> Option<CodexSession> {
        Some(CodexSession {
            name: self.name.clone(),
            touched: self.touched,
        })
    }
}

fn pane(window_id: u64, title: &str, at_prompt: bool, fg: &[&str], cmd: &str) -> Pane {
    Pane {
        window_id,
        root_pid: std::process::id() as i32,
        cwd: "/a/proj".into(),
        title: title.into(),
        at_prompt,
        reported_cmd: Some(cmd.into()),
        foreground_basenames: fg.iter().map(|s| s.to_string()).collect(),
        foreground_pids: vec![],
    }
}

const SELECTION: &str = "\u{276F} 1. Yes\n  2. No\n  enter to confirm";
const CLAUDE_WORKING: &str = "\u{273B} Processing\u{2026} (5s \u{00B7} \u{2193} 1k tokens)";
const CLAUDE_DONE: &str = "\u{273B} Brewed for 1m";

#[test]
fn tick_classifies_codex_blocked_from_screen() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let host = FakeHost {
        panes: vec![pane(11, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: SELECTION.into(),
    };
    tick(&reg, &host, &NoCodexStore, 100);
    let rows = reg.lock().unwrap().sorted();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].status,
        Status::NeedsYou,
        "a selection box on screen => blocked => red"
    );
}

#[test]
fn tick_codex_idle_when_no_turn_taken() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let store = FakeCodex {
        name: Some("Asasdsadasd".into()),
        touched: false,
    };
    let host = FakeHost {
        panes: vec![pane(12, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: String::new(),
    };
    tick(&reg, &host, &store, 100);
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
fn tick_codex_your_turn_when_turn_taken() {
    let reg = Arc::new(Mutex::new(Registry::default()));
    let store = FakeCodex {
        name: Some("Asasdsadasd".into()),
        touched: true,
    };
    let host = FakeHost {
        panes: vec![pane(13, "qol-monorepo", false, &["zsh", "codex"], "codex")],
        screen: String::new(),
    };
    tick(&reg, &host, &store, 100);
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
    tick(&reg, &working, &NoCodexStore, 100);
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
    tick(&reg, &parked, &NoCodexStore, 200);
    assert_eq!(
        reg.lock().unwrap().sorted()[0].status,
        Status::YourTurn,
        "a completion marker means your turn"
    );

    reg.lock().unwrap().get_mut(10).unwrap().acknowledge();
    tick(&reg, &parked, &NoCodexStore, 300);
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
    tick(&reg, &host, &NoCodexStore, 100);
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
    tick(&reg, &host, &NoCodexStore, 100);
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
    tick(&reg, &host, &NoCodexStore, 100);
    let rows = reg.lock().unwrap().sorted();
    assert_eq!(rows[0].name.as_deref(), Some("qol dev"));
    assert_eq!(rows[0].status, Status::Working, "running generic => green");
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
    tick(&reg, &present, &NoCodexStore, 100);
    assert_eq!(reg.lock().unwrap().sorted().len(), 1);

    let gone = FakeHost {
        panes: vec![],
        screen: String::new(),
    };
    tick(&reg, &gone, &NoCodexStore, 200);
    assert_eq!(
        reg.lock().unwrap().sorted().len(),
        0,
        "a window no longer present is removed"
    );
}
