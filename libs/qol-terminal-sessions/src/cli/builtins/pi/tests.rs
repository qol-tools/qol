use std::sync::Arc;

use tempfile::TempDir;

use crate::cli::CliSessionStrategy;
use crate::{BackendId, SessionCapabilities, SessionFacts, SessionId};

use super::environment::PiEnvironment;
use super::PiStrategy;

struct FakeEnvironment {
    session_file: std::path::PathBuf,
}

impl PiEnvironment for FakeEnvironment {
    fn session_file(&self, _cwd: &str) -> Option<std::path::PathBuf> {
        Some(self.session_file.clone())
    }
}

#[test]
fn fresh_session_reads_idle_until_a_message_is_appended() {
    let root = TempDir::new().unwrap();
    let file = root
        .path()
        .join("2026-08-03T09-15-27-264Z_019fc6e8-18a0-7983-9fd6-0200f1e9a72b.jsonl");
    std::fs::write(
        &file,
        "{\"type\":\"session\",\"version\":3,\"id\":\"019fc6e8-18a0-7983-9fd6-0200f1e9a72b\",\"timestamp\":\"2026-08-03T09:15:27.264Z\",\"cwd\":\"/work/proj\"}\n",
    )
    .unwrap();
    let strategy = PiStrategy::with_environment(Arc::new(FakeEnvironment {
        session_file: file.clone(),
    }));

    let fresh = strategy.describe(&session());
    assert_eq!(fresh.has_activity, Some(false));
    assert_eq!(
        fresh.external_id.as_deref(),
        Some("019fc6e8-18a0-7983-9fd6-0200f1e9a72b")
    );
    assert_eq!(fresh.display_name.as_deref(), Some("proj"));

    let mut content = std::fs::read_to_string(&file).unwrap();
    content.push_str("{\"type\":\"message\",\"id\":\"a1\",\"parentId\":null,\"timestamp\":\"2026-08-03T09:16:00.000Z\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n");
    std::fs::write(&file, content).unwrap();

    let active = strategy.describe(&session());
    assert_eq!(active.has_activity, Some(true));
}

#[test]
fn session_info_names_are_picked_up_incrementally() {
    let root = TempDir::new().unwrap();
    let file = root
        .path()
        .join("2026-08-03T09-15-27-264Z_019fc6e8-18a0-7983-9fd6-0200f1e9a72b.jsonl");
    std::fs::write(
        &file,
        "{\"type\":\"session\",\"version\":3,\"id\":\"x\",\"timestamp\":\"t\",\"cwd\":\"/work/proj\"}\n{\"type\":\"message\",\"id\":\"a1\",\"parentId\":null,\"timestamp\":\"t\",\"message\":{\"role\":\"user\",\"content\":[]}}\n",
    )
    .unwrap();
    let strategy = PiStrategy::with_environment(Arc::new(FakeEnvironment {
        session_file: file.clone(),
    }));

    let unnamed = strategy.describe(&session());
    assert_eq!(unnamed.display_name.as_deref(), Some("proj"));

    let mut content = std::fs::read_to_string(&file).unwrap();
    content.push_str("{\"type\":\"session_info\",\"id\":\"k1\",\"parentId\":\"a1\",\"timestamp\":\"t\",\"name\":\"Refactor auth module\"}\n");
    std::fs::write(&file, content).unwrap();

    let named = strategy.describe(&session());
    assert_eq!(named.display_name.as_deref(), Some("Refactor auth module"));
}

#[test]
fn display_name_falls_back_to_the_terminal_title() {
    let root = TempDir::new().unwrap();
    let file = root.path().join("missing.jsonl");
    let strategy = PiStrategy::with_environment(Arc::new(FakeEnvironment { session_file: file }));
    let mut facts = session();
    facts.title = "\u{03C0} - Named work - proj".to_owned();

    let descriptor = strategy.describe(&facts);
    assert_eq!(descriptor.display_name.as_deref(), Some("Named work"));
    assert_eq!(descriptor.has_activity, None);
}

#[test]
fn matches_only_pi_processes() {
    let root = TempDir::new().unwrap();
    let strategy = PiStrategy::with_environment(Arc::new(FakeEnvironment {
        session_file: root.path().join("x.jsonl"),
    }));
    assert!(strategy.matches(&session()));

    let mut other = session();
    other.foreground_basenames = vec!["zsh".to_owned(), "pine".to_owned()];
    assert!(!strategy.matches(&other));
}

fn session() -> SessionFacts {
    SessionFacts {
        id: SessionId::new(BackendId::new("kitty").unwrap(), "7").unwrap(),
        root_pid: 10,
        cwd: "/work/proj".to_owned(),
        title: "\u{03C0} - proj".to_owned(),
        at_prompt: false,
        reported_cmd: Some("pi".to_owned()),
        foreground_basenames: vec!["zsh".to_owned(), "pi".to_owned()],
        foreground_pids: vec![22],
        capabilities: SessionCapabilities::ALL,
    }
}
