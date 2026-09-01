use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;

use crate::cli::CliSessionStrategy;
use crate::cli::{
    CliLaunchProgram, CliRuntimeState, CliScreenEvidence, CliSessionInterpreter, CliViewportState,
};
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

struct EmptyEnvironment;

impl ClaudeEnvironment for EmptyEnvironment {
    fn session(&self, _pid: i32) -> Option<ClaudeSessionLocation> {
        None
    }
}

#[derive(Default)]
struct SwitchableEnvironment {
    location: std::sync::Mutex<Option<ClaudeSessionLocation>>,
    scans: AtomicUsize,
}

impl SwitchableEnvironment {
    fn answer(&self, location: Option<ClaudeSessionLocation>) {
        *self.location.lock().unwrap() = location;
    }

    fn scans(&self) -> usize {
        self.scans.load(Ordering::SeqCst)
    }
}

impl ClaudeEnvironment for SwitchableEnvironment {
    fn session(&self, _pid: i32) -> Option<ClaudeSessionLocation> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        self.location.lock().unwrap().clone()
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
fn transcript_activity_tracks_the_last_write() {
    let root = TempDir::new().unwrap();
    let transcript = root.path().join("session.jsonl");
    std::fs::write(
        &transcript,
        concat!(
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[]},\"timestamp\":\"2026-08-03T09:00:00.000Z\"}\n",
            "{\"type\":\"assistant\",\"message\":{\"role\":\"assistant\",\"content\":[]},\"timestamp\":\"2026-08-03T09:01:00.000Z\"}\n"
        ),
    )
    .unwrap();
    let strategy = ClaudeStrategy::with_environment(Arc::new(FakeEnvironment {
        location: ClaudeSessionLocation {
            external_id: "session-7".to_owned(),
            transcript_path: transcript.clone(),
        },
    }));

    let active = strategy.describe(&session());
    assert_eq!(active.has_activity, Some(true));

    let stale = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(&transcript)
        .unwrap()
        .set_modified(stale)
        .unwrap();

    let idle = strategy.describe(&session());
    assert_eq!(idle.has_activity, Some(false));
}

#[test]
fn transcript_without_messages_reads_idle_even_when_fresh() {
    let root = TempDir::new().unwrap();
    let transcript = root.path().join("session.jsonl");
    std::fs::write(
        &transcript,
        "{\"type\":\"custom-title\",\"customTitle\":\"Only a rename\"}\n",
    )
    .unwrap();
    let strategy = ClaudeStrategy::with_environment(Arc::new(FakeEnvironment {
        location: ClaudeSessionLocation {
            external_id: "session-7".to_owned(),
            transcript_path: transcript,
        },
    }));

    let descriptor = strategy.describe(&session());
    assert_eq!(descriptor.has_activity, Some(false));
}

#[test]
fn missing_transcript_has_no_activity_hint() {
    let root = TempDir::new().unwrap();
    let strategy = ClaudeStrategy::with_environment(Arc::new(FakeEnvironment {
        location: ClaudeSessionLocation {
            external_id: "session-7".to_owned(),
            transcript_path: root.path().join("missing.jsonl"),
        },
    }));

    let descriptor = strategy.describe(&session());
    assert_eq!(descriptor.has_activity, None);
}

#[test]
fn transcript_tail_type_drives_the_runtime() {
    let root = TempDir::new().unwrap();
    let transcript = root.path().join("session.jsonl");
    let strategy = ClaudeStrategy::with_environment(Arc::new(FakeEnvironment {
        location: ClaudeSessionLocation {
            external_id: "session-7".to_owned(),
            transcript_path: transcript.clone(),
        },
    }));

    let interrupt_note = "{\"parentUuid\":\"e418e25b-1b7b-4d21-b78f-cc0612994387\",\"isSidechain\":false,\"promptId\":\"54a79d67-f8c1-450a-b6b4-49e5121e2fb0\",\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"[Request interrupted by user]\"}]},\"uuid\":\"9debe788-2871-44d1-a13f-b4f5a8216e06\",\"timestamp\":\"2026-08-04T20:41:29.417Z\",\"interruptedByShutdown\":true,\"userType\":\"external\",\"entrypoint\":\"sdk-py\",\"cwd\":\"/media/kmrh47/WD_SN850X/Git/qol-monorepo\",\"sessionId\":\"fc0458d8-3c35-4eef-9df5-f9d6a738acdc\",\"version\":\"2.1.221\",\"gitBranch\":\"main\"}\n";
    let cases = [
        ("{\"type\":\"permission-mode\"}\n", CliRuntimeState::Ready),
        ("{\"type\":\"mode\"}\n", CliRuntimeState::Ready),
        ("{\"type\":\"last-prompt\"}\n", CliRuntimeState::Ready),
        (
            "{\"type\":\"system\",\"subtype\":\"turn_duration\"}\n",
            CliRuntimeState::Ready,
        ),
        (
            "{\"type\":\"ai-title\",\"aiTitle\":\"Review build infrastructure migration for security\",\"sessionId\":\"706c6c7e-818a-4619-90b4-c5b87ba8223d\"}\n",
            CliRuntimeState::Ready,
        ),
        (
            "{\"type\":\"atis-latch\",\"atis\":\"\",\"sessionId\":\"66084504-c07b-4e89-95b7-6d351974491c\"}\n",
            CliRuntimeState::Ready,
        ),
        (
            "{\"type\":\"cost-state\",\"sessionId\":\"66084504-c07b-4e89-95b7-6d351974491c\",\"totalCostUSD\":9.576202500000003}\n",
            CliRuntimeState::Ready,
        ),
        (
            "{\"type\":\"artifact-comment-monitor\",\"v\":1,\"sessionId\":\"f6919ab1-e337-47b0-b87d-c79c36c68198\",\"artifacts\":{}}\n",
            CliRuntimeState::Ready,
        ),
        (
            "{\"type\":\"worktree-state\",\"worktreeSession\":{\"originalCwd\":\"/media/kmrh47/WD_SN850X/Git/qol-monorepo\",\"worktreeName\":\"remove-makefiles\"}}\n",
            CliRuntimeState::Ready,
        ),
        (interrupt_note, CliRuntimeState::Ready),
        ("{\"type\":\"user\"}\n", CliRuntimeState::Working),
        ("{\"type\":\"assistant\"}\n", CliRuntimeState::Working),
        ("{\"type\":\"attachment\"}\n", CliRuntimeState::Working),
        (
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"please fix the bug\"}]}}\n",
            CliRuntimeState::Working,
        ),
        (
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"toolu_01\",\"content\":\"ok\"}]}}\n",
            CliRuntimeState::Working,
        ),
        ("complete garbage, not json\n", CliRuntimeState::Working),
    ];
    for (content, runtime) in cases {
        std::fs::write(&transcript, content).unwrap();
        assert_eq!(
            strategy.describe(&session()).evidence.runtime,
            runtime,
            "tail: {content}"
        );
    }
}

#[test]
fn an_unresolved_transcript_location_reads_unknown_not_ready() {
    let strategy = ClaudeStrategy::with_environment(Arc::new(EmptyEnvironment));

    assert_eq!(
        strategy.describe(&session()).evidence.runtime,
        CliRuntimeState::Unknown
    );
}

#[test]
fn a_missed_transcript_lookup_is_retried_after_half_a_second_not_thirty_seconds() {
    let root = TempDir::new().unwrap();
    let transcript = root.path().join("session.jsonl");
    std::fs::write(
        &transcript,
        "{\"type\":\"custom-title\",\"customTitle\":\"Late lane\"}\n",
    )
    .unwrap();
    let environment = Arc::new(SwitchableEnvironment::default());
    let strategy = ClaudeStrategy::with_environment(environment.clone());

    assert_eq!(strategy.metadata.subscription_path(&session()), None);
    assert_eq!(environment.scans(), 1);

    environment.answer(Some(ClaudeSessionLocation {
        external_id: "session-7".to_owned(),
        transcript_path: transcript.clone(),
    }));
    assert_eq!(
        strategy.metadata.subscription_path(&session()),
        None,
        "the stored miss still serves an immediate re-check"
    );
    assert_eq!(
        environment.scans(),
        1,
        "a burst of lookups inside one window shares a single filesystem scan"
    );

    std::thread::sleep(super::metadata::MISSING_SESSION_CACHE_TTL + Duration::from_millis(100));
    assert_eq!(
        strategy.metadata.subscription_path(&session()).as_deref(),
        Some(transcript.as_path())
    );
    assert_eq!(
        environment.scans(),
        2,
        "the expired miss retries instead of serving emptiness for thirty seconds"
    );
}

#[test]
fn a_resolved_transcript_hit_stays_cached_without_another_scan() {
    let root = TempDir::new().unwrap();
    let transcript = root.path().join("session.jsonl");
    std::fs::write(
        &transcript,
        "{\"type\":\"custom-title\",\"customTitle\":\"Cached lane\"}\n",
    )
    .unwrap();
    let environment = Arc::new(SwitchableEnvironment::default());
    let strategy = ClaudeStrategy::with_environment(environment.clone());
    environment.answer(Some(ClaudeSessionLocation {
        external_id: "session-7".to_owned(),
        transcript_path: transcript.clone(),
    }));

    assert_eq!(
        strategy.metadata.subscription_path(&session()).as_deref(),
        Some(transcript.as_path())
    );
    environment.answer(None);
    assert_eq!(
        strategy.metadata.subscription_path(&session()).as_deref(),
        Some(transcript.as_path()),
        "a hit keeps its full cache window even when the answer changes underneath"
    );
    assert_eq!(environment.scans(), 1);
}

#[test]
fn a_missing_transcript_reads_ready_like_a_fresh_session() {
    let root = TempDir::new().unwrap();
    let strategy = ClaudeStrategy::with_environment(Arc::new(FakeEnvironment {
        location: ClaudeSessionLocation {
            external_id: "session-7".to_owned(),
            transcript_path: root.path().join("missing.jsonl"),
        },
    }));

    assert_eq!(
        strategy.describe(&session()).evidence.runtime,
        CliRuntimeState::Ready
    );
}

#[test]
fn an_empty_transcript_reads_ready() {
    let root = TempDir::new().unwrap();
    let transcript = root.path().join("session.jsonl");
    std::fs::write(&transcript, "").unwrap();
    let strategy = ClaudeStrategy::with_environment(Arc::new(FakeEnvironment {
        location: ClaudeSessionLocation {
            external_id: "session-7".to_owned(),
            transcript_path: transcript,
        },
    }));

    assert_eq!(
        strategy.describe(&session()).evidence.runtime,
        CliRuntimeState::Ready
    );
}

#[test]
fn a_trailing_partial_line_does_not_change_the_runtime() {
    let root = TempDir::new().unwrap();
    let transcript = root.path().join("session.jsonl");
    std::fs::write(
        &transcript,
        concat!("{\"type\":\"permission-mode\"}\n", "{\"type\":\"us"),
    )
    .unwrap();
    let strategy = ClaudeStrategy::with_environment(Arc::new(FakeEnvironment {
        location: ClaudeSessionLocation {
            external_id: "session-7".to_owned(),
            transcript_path: transcript,
        },
    }));

    assert_eq!(
        strategy.describe(&session()).evidence.runtime,
        CliRuntimeState::Ready
    );
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

#[test]
fn subscribe_falls_back_to_the_projects_directory_when_no_transcript_exists_yet() {
    let root = TempDir::new().unwrap();
    let projects = root.path().join("projects");
    let project = projects.join("-work-project");
    std::fs::create_dir_all(&project).unwrap();
    let (changed, events) = std::sync::mpsc::channel();
    let mut strategy = ClaudeStrategy::with_environment(Arc::new(EmptyEnvironment));
    strategy.metadata.projects_root = Some(projects);
    let interpreter =
        CliSessionInterpreter::from_strategies([Arc::new(strategy) as Arc<dyn CliSessionStrategy>])
            .unwrap();

    let subscription = interpreter
        .subscribe(
            &session(),
            Arc::new(move || {
                let _ = changed.send(());
            }),
        )
        .unwrap()
        .expect("an empty claude project directory still yields a subscription");

    std::fs::write(project.join("20260803T091527Z_session-7.jsonl"), "{}\n").unwrap();

    events
        .recv_timeout(std::time::Duration::from_secs(3))
        .expect("a transcript appearing in the project directory wakes the subscription");
    drop(subscription);
}

#[test]
fn launch_program_is_the_claude_executable_without_arguments() {
    assert_eq!(
        ClaudeStrategy::default().launch(),
        Some(CliLaunchProgram::new("claude"))
    );
}

#[test]
fn screen_classification_distinguishes_work_done_and_plain_output() {
    let strategy = ClaudeStrategy::default();
    let facts = session();

    assert_eq!(
        strategy.classify_screen(&facts, "esc to interrupt"),
        CliScreenEvidence {
            viewport: CliViewportState::Live,
            runtime: CliRuntimeState::Working,
        }
    );
    assert_eq!(
        strategy.classify_screen(&facts, "\u{2728} Running tests \u{2026} (12s)"),
        CliScreenEvidence {
            viewport: CliViewportState::Live,
            runtime: CliRuntimeState::Working,
        }
    );
    assert_eq!(
        strategy.classify_screen(&facts, "plain output"),
        CliScreenEvidence::default()
    );
}

#[test]
fn claude_done_marker_sets_runtime_ready_but_never_a_live_viewport() {
    let strategy = ClaudeStrategy::default();
    let facts = session();

    let done = strategy.classify_screen(&facts, "\u{2734} Fixed the queue for 12s");
    assert_eq!(done.runtime, CliRuntimeState::Ready);
    assert_eq!(done.viewport, CliViewportState::Unknown);

    let scrollback = format!(
        "\u{2734} Fixed the queue for 12s\n{}",
        "filler\n".repeat(20)
    );
    let stale_scrollback = strategy.classify_screen(&facts, &scrollback);
    assert_eq!(stale_scrollback.runtime, CliRuntimeState::Unknown);
    assert_eq!(stale_scrollback.viewport, CliViewportState::Unknown);
}

#[test]
fn the_startup_placeholder_title_falls_back_to_the_spawn_key() {
    let root = TempDir::new().unwrap();
    let transcript = root.path().join("missing.jsonl");
    let strategy = ClaudeStrategy::with_environment(Arc::new(FakeEnvironment {
        location: ClaudeSessionLocation {
            external_id: "session-7".to_owned(),
            transcript_path: transcript,
        },
    }));

    let mut facts = session();
    facts.title = "\u{2733} Claude Code".to_owned();
    facts.spawn_identity = Some(crate::SpawnIdentity {
        key: crate::SpawnKey::new("titlecheck-claude").unwrap(),
        tool: crate::cli::CliToolId::new("claude").unwrap(),
        surface: crate::SpawnSurface::Tab,
    });

    assert_eq!(
        strategy.describe(&facts).display_name.as_deref(),
        Some("titlecheck-claude")
    );
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
        spawn_identity: None,
    }
}
