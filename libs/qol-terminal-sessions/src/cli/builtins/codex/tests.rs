use std::sync::Arc;

use tempfile::TempDir;

use crate::cli::CliSessionStrategy;
use crate::cli::{
    CliLaunchProgram, CliRuntimeState, CliScreenEvidence, CliSessionEvidence, CliViewportState,
};
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

#[test]
fn completion_subscription_tracks_the_target_rollout() {
    let root = TempDir::new().unwrap();
    let rollout = root.path().join("rollout.jsonl");
    let index = root.path().join("session_index.jsonl");
    std::fs::write(&rollout, "first\nsecond\n").unwrap();
    std::fs::write(&index, "").unwrap();
    let strategy = CodexStrategy::with_environment(Arc::new(FakeEnvironment {
        rollout: rollout.clone(),
        index,
    }));

    assert_eq!(
        strategy.metadata.subscription_path(&session()),
        Some(rollout)
    );
}

#[test]
fn title_activity_and_thread_name_follow_the_default_title_layout() {
    let root = TempDir::new().unwrap();
    let id = "019f9dd4-ef90-7a43-9ae0-ca1c2b5d8d6a";
    let rollout = root.path().join(format!("rollout-{id}.jsonl"));
    std::fs::write(&rollout, "\n").unwrap();
    let index = root.path().join("session_index.jsonl");
    std::fs::write(&index, "").unwrap();
    let strategy = CodexStrategy::with_environment(Arc::new(FakeEnvironment {
        rollout: rollout.clone(),
        index,
    }));

    let cases = [
        (
            "qol-tts | Working | fix the queue",
            Some(true),
            Some("fix the queue"),
        ),
        (
            "qol-tts | Thinking | hard problem",
            Some(true),
            Some("hard problem"),
        ),
        ("qol-tts | Action Required | qol-tts", Some(false), None),
        ("qol-tts | Ready | qol-tts", Some(false), None),
        ("qol-tts | Ready | gpt-5.6-luna max", Some(false), None),
        ("qol-tts | Ready | ", Some(false), None),
        ("qol-tts | Ready | \u{1}", Some(false), None),
        ("qol-tts", Some(false), None),
    ];

    for (title, activity, thread_name) in cases {
        let mut facts = session();
        facts.title = title.to_owned();
        let descriptor = strategy.describe(&facts);
        assert_eq!(descriptor.has_activity, activity, "title: {title}");
        let expected = thread_name
            .map(str::to_owned)
            .or(Some("qol-tts".to_owned()));
        assert_eq!(descriptor.display_name, expected, "title: {title}");
    }
}

#[test]
fn the_session_index_name_wins_over_the_trailing_title_segment() {
    let root = TempDir::new().unwrap();
    let id = "019f9dd4-ef90-7a43-9ae0-ca1c2b5d8d6a";
    let rollout = root.path().join(format!("rollout-{id}.jsonl"));
    std::fs::write(&rollout, "first\nsecond\n").unwrap();
    let index = root.path().join("session_index.jsonl");
    std::fs::write(
        &index,
        format!(r#"{{"id":"{id}","thread_name":"kcd2-implementor"}}"#),
    )
    .unwrap();
    let strategy = CodexStrategy::with_environment(Arc::new(FakeEnvironment { rollout, index }));

    let mut facts = session();
    facts.title = "kcd2-implementor | Ready | gpt-5.6-luna max".to_owned();

    assert_eq!(
        strategy.describe(&facts).display_name.as_deref(),
        Some("kcd2-implementor")
    );
}

#[test]
fn a_leading_title_name_wins_and_an_unnamed_thread_falls_back_to_the_spawn_key() {
    let root = TempDir::new().unwrap();
    let id = "019f9dd4-ef90-7a43-9ae0-ca1c2b5d8d6a";
    let rollout = root.path().join(format!("rollout-{id}.jsonl"));
    std::fs::write(&rollout, "first\nsecond\n").unwrap();
    let index = root.path().join("session_index.jsonl");
    std::fs::write(&index, "").unwrap();
    let strategy = CodexStrategy::with_environment(Arc::new(FakeEnvironment { rollout, index }));

    let mut named = session();
    named.title = "kcd2-implementor | Ready | gpt-5.6-luna max".to_owned();
    assert_eq!(
        strategy.describe(&named).display_name.as_deref(),
        Some("kcd2-implementor")
    );

    let mut unnamed = session();
    unnamed.title = format!("{id} | Ready | gpt-5.6-sol high");
    unnamed.spawn_identity = Some(crate::SpawnIdentity {
        key: crate::SpawnKey::new("titlecheck-codex").unwrap(),
        tool: crate::cli::CliToolId::new("codex").unwrap(),
        surface: crate::SpawnSurface::Tab,
    });
    assert_eq!(
        strategy.describe(&unnamed).display_name.as_deref(),
        Some("titlecheck-codex")
    );
}

#[test]
fn title_activity_falls_back_to_rollout_state_for_unknown_titles() {
    let root = TempDir::new().unwrap();
    let id = "019f9dd4-ef90-7a43-9ae0-ca1c2b5d8d6a";
    let rollout = root.path().join(format!("rollout-{id}.jsonl"));
    std::fs::write(
        &rollout,
        "{\"type\":\"item_start\"}\n{\"type\":\"response_item\"}\n",
    )
    .unwrap();
    let index = root.path().join("session_index.jsonl");
    std::fs::write(&index, "").unwrap();
    let strategy = CodexStrategy::with_environment(Arc::new(FakeEnvironment { rollout, index }));

    let mut facts = session();
    facts.title = "some unrelated title".to_owned();
    assert_eq!(strategy.describe(&facts).has_activity, Some(true));
}

#[test]
fn rollout_fallback_activity_goes_idle_when_the_rollout_stops_being_written() {
    let root = TempDir::new().unwrap();
    let id = "019f9dd4-ef90-7a43-9ae0-ca1c2b5d8d6a";
    let rollout = root.path().join(format!("rollout-{id}.jsonl"));
    std::fs::write(
        &rollout,
        "{\"type\":\"item_start\"}\n{\"type\":\"response_item\"}\n",
    )
    .unwrap();
    let index = root.path().join("session_index.jsonl");
    std::fs::write(&index, "").unwrap();
    let strategy = CodexStrategy::with_environment(Arc::new(FakeEnvironment {
        rollout: rollout.clone(),
        index,
    }));

    let mut facts = session();
    facts.title = "some unrelated title".to_owned();
    assert_eq!(strategy.describe(&facts).has_activity, Some(true));

    let stale = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(&rollout)
        .unwrap()
        .set_modified(stale)
        .unwrap();
    assert_eq!(strategy.describe(&facts).has_activity, Some(false));
}

#[test]
fn descriptor_evidence_derives_strong_runtime_from_title_markers_only() {
    let root = TempDir::new().unwrap();
    let id = "019f9dd4-ef90-7a43-9ae0-ca1c2b5d8d6a";
    let rollout = root.path().join(format!("rollout-{id}.jsonl"));
    std::fs::write(&rollout, "first\nsecond\n").unwrap();
    let index = root.path().join("session_index.jsonl");
    std::fs::write(&index, "").unwrap();
    let strategy = CodexStrategy::with_environment(Arc::new(FakeEnvironment { rollout, index }));

    let cases = [
        (
            "qol-tts | Working | fix the queue",
            CliRuntimeState::Working,
        ),
        (
            "qol-tts | Thinking | hard problem",
            CliRuntimeState::Working,
        ),
        ("qol-tts | Ready | qol-tts", CliRuntimeState::Ready),
        (
            "qol-tts | Action Required | qol-tts",
            CliRuntimeState::Unknown,
        ),
        (
            "qol-tts | Action Required | Ready | gpt-5.6-luna max",
            CliRuntimeState::Ready,
        ),
        ("qol-tts", CliRuntimeState::Unknown),
    ];
    for (title, runtime) in cases {
        let mut facts = session();
        facts.title = title.to_owned();
        let evidence = strategy.describe(&facts).evidence;
        assert_eq!(evidence.runtime, runtime, "title: {title}");
        assert_eq!(evidence.activity.file_fresh, Some(true), "title: {title}");
        assert_eq!(
            evidence.activity.file_has_work,
            Some(true),
            "title: {title}"
        );
        assert_eq!(
            evidence,
            CliSessionEvidence {
                runtime,
                activity: evidence.activity,
            },
            "title: {title}"
        );
    }
}

#[test]
fn metadata_attachment_never_proves_live_and_weak_evidence_stays_out_of_runtime() {
    let root = TempDir::new().unwrap();
    let id = "019f9dd4-ef90-7a43-9ae0-ca1c2b5d8d6a";
    let rollout = root.path().join(format!("rollout-{id}.jsonl"));
    std::fs::write(&rollout, "first\nsecond\n").unwrap();
    let index = root.path().join("session_index.jsonl");
    std::fs::write(&index, "").unwrap();
    let strategy = CodexStrategy::with_environment(Arc::new(FakeEnvironment {
        rollout: rollout.clone(),
        index,
    }));
    let mut facts = session();
    facts.title = "some unrelated title".to_owned();
    let descriptor = strategy.describe(&facts);
    assert_eq!(descriptor.external_id.as_deref(), Some(id));
    assert_eq!(descriptor.evidence.runtime, CliRuntimeState::Unknown);

    let stale = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(&rollout)
        .unwrap()
        .set_modified(stale)
        .unwrap();
    let evidence = strategy.describe(&facts).evidence;
    assert_eq!(evidence.activity.file_fresh, Some(false));
    assert_eq!(evidence.activity.file_has_work, Some(true));
    assert_eq!(evidence.activity.combined(), Some(false));
    assert_eq!(evidence.runtime, CliRuntimeState::Unknown);
}

#[test]
fn launch_program_is_the_codex_executable_without_arguments() {
    let strategy = CodexStrategy::default();
    assert_eq!(strategy.launch(), Some(CliLaunchProgram::new("codex")));
}

#[test]
fn screen_classification_distinguishes_work_prompts_and_banners() {
    let strategy = CodexStrategy::default();
    let facts = session();

    assert_eq!(
        strategy.classify_screen(&facts, "  esc to interrupt "),
        CliScreenEvidence {
            viewport: CliViewportState::Live,
            runtime: CliRuntimeState::Working,
        }
    );
    assert_eq!(
        strategy.classify_screen(&facts, "1) run tests\nenter to accept"),
        CliScreenEvidence {
            viewport: CliViewportState::Live,
            runtime: CliRuntimeState::NeedsInput,
        }
    );
    assert_eq!(
        strategy.classify_screen(&facts, "OpenAI Codex (v0.40)\nTip: Try the Codex App"),
        CliScreenEvidence {
            viewport: CliViewportState::Historical,
            runtime: CliRuntimeState::Unknown,
        }
    );
    assert_eq!(
        strategy.classify_screen(&facts, "plain output"),
        CliScreenEvidence::default()
    );
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
        spawn_identity: None,
    }
}
