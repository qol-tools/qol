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
    fn session_file(&self, _pid: i32, _cwd: &str) -> Option<std::path::PathBuf> {
        Some(self.session_file.clone())
    }
}

struct PerPidEnvironment {
    session_files: std::collections::HashMap<i32, std::path::PathBuf>,
}

impl PiEnvironment for PerPidEnvironment {
    fn session_file(&self, pid: i32, _cwd: &str) -> Option<std::path::PathBuf> {
        self.session_files.get(&pid).cloned()
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
fn two_panes_in_one_directory_keep_their_own_session_names() {
    let root = TempDir::new().unwrap();
    let named = |file: &std::path::Path, name: &str| {
        std::fs::write(
            file,
            format!(
                "{{\"type\":\"session\",\"version\":3,\"id\":\"x\",\"timestamp\":\"t\",\"cwd\":\"/work/proj\"}}\n{{\"type\":\"session_info\",\"id\":\"k1\",\"parentId\":null,\"timestamp\":\"t\",\"name\":\"{name}\"}}\n"
            ),
        )
        .unwrap();
    };
    let first = root
        .path()
        .join("2026-08-03T12-33-49-576Z_019fc79d-b608-7dd5-83c0-af0e4691150a.jsonl");
    let second = root
        .path()
        .join("2026-08-03T12-37-45-895Z_019fc7a1-5126-7ae6-8749-0d2c7688c6ad.jsonl");
    named(&first, "Bug Hunt");
    named(&second, "Headless CLI research");

    let strategy = PiStrategy::with_environment(Arc::new(PerPidEnvironment {
        session_files: [(22, first), (23, second)].into_iter().collect(),
    }));

    let mut bug_hunt = session();
    bug_hunt.foreground_pids = vec![22];
    let mut research = session();
    research.foreground_pids = vec![23];

    assert_eq!(
        strategy.describe(&bug_hunt).display_name.as_deref(),
        Some("Bug Hunt")
    );
    assert_eq!(
        strategy.describe(&research).display_name.as_deref(),
        Some("Headless CLI research"),
        "renaming one pane must not rename the other"
    );
    assert_eq!(
        strategy.describe(&bug_hunt).external_id.as_deref(),
        Some("019fc79d-b608-7dd5-83c0-af0e4691150a")
    );
}

#[test]
fn a_resumed_pane_keeps_its_own_name_when_the_resolver_claims_a_neighbour_file() {
    let root = TempDir::new().unwrap();
    let neighbour = root
        .path()
        .join("2026-08-03T15-05-39-502Z_019fc828-b7ae-78b9-8d02-7a557cf1f504.jsonl");
    std::fs::write(
        &neighbour,
        "{\"type\":\"session\",\"version\":3,\"id\":\"x\",\"timestamp\":\"t\",\"cwd\":\"/work/proj\"}\n{\"type\":\"session_info\",\"id\":\"k1\",\"parentId\":null,\"timestamp\":\"t\",\"name\":\"cli sessions improvements\"}\n",
    )
    .unwrap();
    let strategy = PiStrategy::with_environment(Arc::new(FakeEnvironment {
        session_file: neighbour,
    }));

    let mut resumed = session();
    resumed.title = "\u{03C0} - Bug Hunt - proj".to_owned();

    assert_eq!(
        strategy.describe(&resumed).display_name.as_deref(),
        Some("Bug Hunt"),
        "the pane's own title outranks a session file it does not own"
    );
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
