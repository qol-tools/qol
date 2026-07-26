use std::sync::Arc;

use tempfile::TempDir;

use crate::cli::CliSessionStrategy;
use crate::{BackendId, SessionCapabilities, SessionFacts, SessionId};

use super::environment::CodexEnvironment;
use super::CodexStrategy;

struct FakeEnvironment {
    rollout: std::path::PathBuf,
    index: std::path::PathBuf,
}

impl CodexEnvironment for FakeEnvironment {
    fn open_rollout(&self, _pid: i32) -> Option<std::path::PathBuf> {
        Some(self.rollout.clone())
    }

    fn session_index_path(&self) -> Option<std::path::PathBuf> {
        Some(self.index.clone())
    }
}

#[test]
fn session_index_changes_refresh_names_without_waiting_for_a_ttl() {
    let root = TempDir::new().unwrap();
    let id = "019f9dd4-ef90-7a43-9ae0-ca1c2b5d8d6a";
    let rollout = root.path().join(format!("rollout-{id}.jsonl"));
    let index = root.path().join("session_index.jsonl");
    std::fs::write(&rollout, "first\nsecond\n").unwrap();
    std::fs::write(
        &index,
        format!(r#"{{"id":"{id}","thread_name":"Old name"}}"#),
    )
    .unwrap();
    let strategy = CodexStrategy::with_environment(Arc::new(FakeEnvironment {
        rollout,
        index: index.clone(),
    }));

    let first = strategy.describe(&session());
    std::fs::write(
        index,
        format!(
            "{{\"id\":\"{id}\",\"thread_name\":\"Old name\"}}\n{{\"id\":\"{id}\",\"thread_name\":\"QoL Voice and other improvements\"}}"
        ),
    )
    .unwrap();
    let renamed = strategy.describe(&session());

    assert_eq!(first.display_name.as_deref(), Some("Old name"));
    assert_eq!(
        renamed.display_name.as_deref(),
        Some("QoL Voice and other improvements")
    );
    assert_eq!(renamed.external_id.as_deref(), Some(id));
    assert_eq!(renamed.has_activity, Some(true));
}

fn session() -> SessionFacts {
    SessionFacts {
        id: SessionId::new(BackendId::new("kitty").unwrap(), "7").unwrap(),
        root_pid: 10,
        cwd: "/work/qol-tts".to_owned(),
        title: "qol-tts".to_owned(),
        at_prompt: false,
        reported_cmd: Some("codex".to_owned()),
        foreground_basenames: vec!["zsh".to_owned(), "codex".to_owned()],
        foreground_pids: vec![22],
        capabilities: SessionCapabilities::ALL,
    }
}
