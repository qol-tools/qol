use std::path::PathBuf;
use std::sync::Arc;

use tempfile::TempDir;

use crate::cli::CliSessionStrategy;
use crate::{BackendId, SessionCapabilities, SessionFacts, SessionId};

use super::environment::{KimiEnvironment, KimiSessionLocation};
use super::KimiStrategy;

struct FakeEnvironment {
    location: Option<KimiSessionLocation>,
}

impl KimiEnvironment for FakeEnvironment {
    fn session(&self, _cwd: &str) -> Option<KimiSessionLocation> {
        self.location.clone()
    }
}

#[test]
fn fresh_session_reads_idle_until_a_prompt_is_sent() {
    let root = TempDir::new().unwrap();
    let state = root.path().join("state.json");
    std::fs::write(
        &state,
        r#"{"createdAt":"2026-08-03T09:06:52.000Z","updatedAt":"2026-08-03T09:06:52.000Z","title":"New Session","isCustomTitle":false,"workDir":"/work/proj","lastPrompt":""}"#,
    )
    .unwrap();
    let strategy = strategy(state.clone(), "session_abc-123");
    let session = session();

    let fresh = strategy.describe(&session);
    assert_eq!(fresh.has_activity, Some(false));
    assert_eq!(fresh.external_id.as_deref(), Some("session_abc-123"));
    assert_eq!(fresh.display_name.as_deref(), Some("proj"));

    std::fs::write(
        &state,
        r#"{"createdAt":"2026-08-03T09:06:52.000Z","updatedAt":"2026-08-03T09:15:00.000Z","title":"Refactor auth module","isCustomTitle":false,"workDir":"/work/proj","lastPrompt":"refactor auth"}"#,
    )
    .unwrap();

    let active = strategy.describe(&session);
    assert_eq!(active.has_activity, Some(true));
    assert_eq!(active.display_name.as_deref(), Some("Refactor auth module"));
}

#[test]
fn session_name_is_none_when_state_is_missing() {
    let root = TempDir::new().unwrap();
    let strategy = strategy(root.path().join("missing.json"), "session_x");

    let descriptor = strategy.describe(&session());
    assert_eq!(descriptor.display_name.as_deref(), Some("proj"));
    assert_eq!(descriptor.external_id.as_deref(), Some("session_x"));
    assert_eq!(descriptor.has_activity, None);
}

#[test]
fn same_directory_panes_do_not_share_kimi_session_metadata() {
    let root = TempDir::new().unwrap();
    let state = root.path().join("state.json");
    std::fs::write(&state, r#"{"title":"One pane","lastPrompt":"work"}"#).unwrap();
    let strategy = strategy(state, "session_x");
    let first = session();
    let mut second = session();
    second.id = SessionId::new(BackendId::new("kitty").unwrap(), "8").unwrap();
    second.root_pid = 11;
    second.foreground_pids = vec![23];

    assert_eq!(
        strategy.describe(&first).external_id.as_deref(),
        Some("session_x")
    );
    let second_descriptor = strategy.describe(&second);
    let first_descriptor = strategy.describe(&first);
    for descriptor in [first_descriptor, second_descriptor] {
        assert_eq!(descriptor.external_id, None);
        assert_eq!(descriptor.display_name.as_deref(), Some("proj"));
        assert_eq!(descriptor.has_activity, None);
    }
    assert!(strategy
        .subscribe(&first, Arc::new(|| {}))
        .unwrap()
        .is_none());
}

#[test]
fn matches_only_kimi_processes() {
    let root = TempDir::new().unwrap();
    let strategy = strategy(root.path().join("state.json"), "session_x");
    assert!(strategy.matches(&session()));

    for other in ["pine", "kimi-tool", "claude"] {
        let mut facts = session();
        facts.foreground_basenames = vec!["zsh".to_owned(), other.to_owned()];
        assert!(
            !strategy.matches(&facts),
            "process `{other}` must not match kimi"
        );
    }

    for process in ["kimi", "kimi-co", "kimi-code"] {
        let mut facts = session();
        facts.foreground_basenames = vec!["zsh".to_owned(), process.to_owned()];
        assert!(strategy.matches(&facts), "process `{process}` must match");
    }
}

fn strategy(state_path: PathBuf, session_id: &str) -> KimiStrategy {
    KimiStrategy::with_environment(Arc::new(FakeEnvironment {
        location: Some(KimiSessionLocation {
            session_id: session_id.to_owned(),
            state_path,
        }),
    }))
}

fn session() -> SessionFacts {
    SessionFacts {
        id: SessionId::new(BackendId::new("kitty").unwrap(), "7").unwrap(),
        root_pid: 10,
        cwd: "/work/proj".to_owned(),
        title: "proj".to_owned(),
        at_prompt: false,
        reported_cmd: Some("kimi".to_owned()),
        foreground_basenames: vec!["zsh".to_owned(), "kimi-code".to_owned()],
        foreground_pids: vec![22],
        capabilities: SessionCapabilities::ALL,
    }
}
