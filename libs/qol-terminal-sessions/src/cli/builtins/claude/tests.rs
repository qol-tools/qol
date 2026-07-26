use std::sync::Arc;

use tempfile::TempDir;

use crate::cli::CliSessionStrategy;
use crate::{BackendId, SessionCapabilities, SessionFacts, SessionId};

use super::environment::{ClaudeEnvironment, ClaudeSessionLocation};
use super::{clean_title, ClaudeStrategy};

struct FakeEnvironment {
    location: ClaudeSessionLocation,
}

impl ClaudeEnvironment for FakeEnvironment {
    fn session(&self, _pid: i32) -> Option<ClaudeSessionLocation> {
        Some(self.location.clone())
    }
}

#[test]
fn transcript_title_changes_refresh_semantic_identity() {
    let root = TempDir::new().unwrap();
    let transcript = root.path().join("session.jsonl");
    std::fs::write(
        &transcript,
        "{\"type\":\"custom-title\",\"customTitle\":\"Old name\"}\n",
    )
    .unwrap();
    let strategy = ClaudeStrategy::with_environment(Arc::new(FakeEnvironment {
        location: ClaudeSessionLocation {
            external_id: "session-7".to_owned(),
            transcript_path: transcript.clone(),
        },
    }));

    let first = strategy.describe(&session());
    std::fs::write(
        transcript,
        concat!(
            "{\"type\":\"custom-title\",\"customTitle\":\"Old name\"}\n",
            "{\"type\":\"custom-title\",\"customTitle\":\"New name\"}\n"
        ),
    )
    .unwrap();
    let renamed = strategy.describe(&session());

    assert_eq!(first.display_name.as_deref(), Some("Old name"));
    assert_eq!(renamed.display_name.as_deref(), Some("New name"));
    assert_eq!(renamed.external_id.as_deref(), Some("session-7"));
}

#[test]
fn claude_title_cleanup_preserves_the_semantic_name() {
    let cases = [
        ("✳ Improve logging", "Improve logging"),
        ("  ⠋ Build feature", "Build feature"),
        ("Plain title", "Plain title"),
    ];

    for (title, expected) in cases {
        assert_eq!(clean_title(title).as_deref(), Some(expected));
    }
}

#[test]
fn claude_strategy_exposes_its_transcript_subscription() {
    let root = TempDir::new().unwrap();
    let transcript = root.path().join("session.jsonl");
    std::fs::write(&transcript, "{}\n").unwrap();
    let strategy = ClaudeStrategy::with_environment(Arc::new(FakeEnvironment {
        location: ClaudeSessionLocation {
            external_id: "session-7".to_owned(),
            transcript_path: transcript,
        },
    }));

    let subscription = strategy.subscribe(&session(), Arc::new(|| {})).unwrap();

    assert!(subscription.is_some());
}

fn session() -> SessionFacts {
    SessionFacts {
        id: SessionId::new(BackendId::new("kitty").unwrap(), "7").unwrap(),
        root_pid: 10,
        cwd: "/work/project".to_owned(),
        title: "Claude".to_owned(),
        at_prompt: false,
        reported_cmd: Some("claude".to_owned()),
        foreground_basenames: vec!["zsh".to_owned(), "claude".to_owned()],
        foreground_pids: vec![22],
        capabilities: SessionCapabilities::ALL,
    }
}
