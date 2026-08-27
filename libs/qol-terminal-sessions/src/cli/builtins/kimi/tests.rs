use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tempfile::TempDir;

use crate::cli::CliSessionStrategy;
use crate::cli::{
    CliLaunchProgram, CliRuntimeState, CliScreenEvidence, CliSessionEvidence,
    CliSessionInterpreter, CliViewportState,
};
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

#[derive(Default)]
struct SwitchableEnvironment {
    location: std::sync::Mutex<Option<KimiSessionLocation>>,
    scans: AtomicUsize,
}

impl SwitchableEnvironment {
    fn answer(&self, location: Option<KimiSessionLocation>) {
        *self.location.lock().unwrap() = location;
    }

    fn scans(&self) -> usize {
        self.scans.load(Ordering::SeqCst)
    }
}

impl KimiEnvironment for SwitchableEnvironment {
    fn session(&self, _cwd: &str) -> Option<KimiSessionLocation> {
        self.scans.fetch_add(1, Ordering::SeqCst);
        self.location.lock().unwrap().clone()
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
fn activity_goes_idle_when_the_session_stops_being_written() {
    let root = TempDir::new().unwrap();
    let state = root.path().join("state.json");
    std::fs::write(
        &state,
        r#"{"createdAt":"2026-08-03T09:06:52.000Z","updatedAt":"2026-08-03T09:15:00.000Z","title":"Refactor auth module","isCustomTitle":false,"workDir":"/work/proj","lastPrompt":"refactor auth"}"#,
    )
    .unwrap();
    let strategy = strategy(state.clone(), "session_abc-123");

    let active = strategy.describe(&session());
    assert_eq!(active.has_activity, Some(true));

    let stale = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(&state)
        .unwrap()
        .set_modified(stale)
        .unwrap();

    let idle = strategy.describe(&session());
    assert_eq!(idle.has_activity, Some(false));
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
fn a_missed_state_file_lookup_is_retried_after_half_a_second_not_thirty_seconds() {
    let root = TempDir::new().unwrap();
    let state = root.path().join("state.json");
    std::fs::write(
        &state,
        r#"{"createdAt":"t","updatedAt":"t","title":"Fresh lane","workDir":"/work/proj","lastPrompt":"go"}"#,
    )
    .unwrap();
    let environment = Arc::new(SwitchableEnvironment::default());
    let strategy = KimiStrategy::with_environment(environment.clone());

    assert_eq!(strategy.metadata.subscription_path(&session()), None);
    assert_eq!(environment.scans(), 1);

    environment.answer(Some(KimiSessionLocation {
        session_id: "session_abc-123".to_owned(),
        state_path: state.clone(),
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
        Some(state.as_path())
    );
    assert_eq!(
        environment.scans(),
        2,
        "the expired miss retries instead of serving emptiness for thirty seconds"
    );
}

#[test]
fn a_resolved_state_file_hit_stays_cached_without_another_scan() {
    let root = TempDir::new().unwrap();
    let state = root.path().join("state.json");
    std::fs::write(
        &state,
        r#"{"createdAt":"t","updatedAt":"t","title":"Cached lane","workDir":"/work/proj","lastPrompt":"go"}"#,
    )
    .unwrap();
    let environment = Arc::new(SwitchableEnvironment::default());
    let strategy = KimiStrategy::with_environment(environment.clone());
    environment.answer(Some(KimiSessionLocation {
        session_id: "session_abc-123".to_owned(),
        state_path: state.clone(),
    }));

    assert_eq!(
        strategy.metadata.subscription_path(&session()).as_deref(),
        Some(state.as_path())
    );
    environment.answer(None);
    assert_eq!(
        strategy.metadata.subscription_path(&session()).as_deref(),
        Some(state.as_path()),
        "a hit keeps its full cache window even when the answer changes underneath"
    );
    assert_eq!(environment.scans(), 1);
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

#[test]
fn subscribe_falls_back_to_the_session_group_when_no_state_file_exists_yet() {
    let root = TempDir::new().unwrap();
    let group = root.path().join("wd_proj_abc");
    std::fs::create_dir_all(&group).unwrap();
    std::fs::write(
        root.path().join("session_index.jsonl"),
        format!(
            r#"{{"sessionId":"session_old","sessionDir":"{}/session_old","workDir":"/work/proj"}}"#,
            group.display()
        ),
    )
    .unwrap();
    let (changed, events) = std::sync::mpsc::channel();
    let previous_home = std::env::var_os("KIMI_CODE_HOME");
    std::env::set_var("KIMI_CODE_HOME", root.path());
    let interpreter = CliSessionInterpreter::from_strategies([
        Arc::new(KimiStrategy::default()) as Arc<dyn CliSessionStrategy>
    ])
    .unwrap();

    let subscription_result = interpreter.subscribe(
        &session(),
        Arc::new(move || {
            let _ = changed.send(());
        }),
    );
    match previous_home {
        Some(home) => std::env::set_var("KIMI_CODE_HOME", home),
        None => std::env::remove_var("KIMI_CODE_HOME"),
    }

    let subscription = subscription_result
        .unwrap()
        .expect("a kimi cwd with a known but file-less session group still yields a subscription");

    let fresh = group.join("session_new");
    std::fs::create_dir(&fresh).unwrap();
    std::fs::write(fresh.join("state.json"), r#"{"title":"Fresh lane"}"#).unwrap();

    events
        .recv_timeout(std::time::Duration::from_secs(3))
        .expect("a new session directory under the group wakes the subscription");
    drop(subscription);
}

fn strategy(state_path: PathBuf, session_id: &str) -> KimiStrategy {
    KimiStrategy::with_environment(Arc::new(FakeEnvironment {
        location: Some(KimiSessionLocation {
            session_id: session_id.to_owned(),
            state_path,
        }),
    }))
}

#[test]
fn launch_program_is_the_kimi_executable_without_arguments() {
    assert_eq!(
        KimiStrategy::default().launch(),
        Some(CliLaunchProgram::new("kimi"))
    );
}

fn kimi_screen(tail: &str) -> String {
    format!(
        "{tail}\nyolo  K3-256k thinking: low  \u{2026}/qol-monorepo  main [\u{00B1}]\ncontext: 17% (41.1k/256k)"
    )
}

#[test]
fn screen_classification_distinguishes_work_and_questionnaires() {
    let strategy = KimiStrategy::default();
    let facts = session();

    assert_eq!(
        strategy.classify_screen(&facts, &kimi_screen("\u{1F311}\u{FE0F} \u{00B7} building")),
        CliScreenEvidence {
            viewport: CliViewportState::Live,
            runtime: CliRuntimeState::Working,
        }
    );
    assert_eq!(
        strategy.classify_screen(
            &facts,
            &kimi_screen("1) quick\n2) thorough\ntab switch to change, esc cancel, \u{21B5} save")
        ),
        CliScreenEvidence {
            viewport: CliViewportState::Live,
            runtime: CliRuntimeState::NeedsInput,
        }
    );
    assert_eq!(
        strategy.classify_screen(
            &facts,
            &kimi_screen(
                "    [1] Submit\n    [2] Cancel\n  \u{2191}\u{2193} select  1/2 choose  \u{21B5} confirm  tab switch  esc cancel"
            ),
        ),
        CliScreenEvidence {
            viewport: CliViewportState::Live,
            runtime: CliRuntimeState::NeedsInput,
        }
    );
    assert_eq!(
        strategy.classify_screen(
            &facts,
            &kimi_screen(
                "  \u{25B6} 1. Allow\n    2. Deny\n  \u{2191}\u{2193} select \u{00B7} 1/2 choose \u{00B7} \u{21B5} confirm"
            ),
        ),
        CliScreenEvidence {
            viewport: CliViewportState::Live,
            runtime: CliRuntimeState::NeedsInput,
        }
    );
    assert_eq!(
        strategy.classify_screen(
            &facts,
            "older transcript line\n  \u{2191}\u{2193} select  1/2 choose  \u{21B5} confirm"
        ),
        CliScreenEvidence::default(),
        "a panned frame ending in a stray hint must hold instead of misreading"
    );
    assert_eq!(
        strategy.classify_screen(&facts, "plain output"),
        CliScreenEvidence::default()
    );
}

#[test]
fn metadata_attachment_never_proves_live_for_kimi() {
    let root = TempDir::new().unwrap();
    let state = root.path().join("state.json");
    std::fs::write(
        &state,
        r#"{"title":"Refactor the queue","lastPrompt":"go"}"#,
    )
    .unwrap();
    let strategy = strategy(state, "session-9");

    let descriptor = strategy.describe(&session());
    assert_eq!(descriptor.external_id.as_deref(), Some("session-9"));
    assert_eq!(descriptor.evidence.runtime, CliRuntimeState::Unknown);
    assert_eq!(descriptor.evidence.activity.file_has_work, Some(true));
    assert_eq!(
        descriptor.evidence,
        CliSessionEvidence {
            runtime: CliRuntimeState::Unknown,
            activity: descriptor.evidence.activity,
        }
    );
}

#[test]
fn a_prompt_length_auto_title_falls_back_to_the_spawn_key() {
    let root = TempDir::new().unwrap();
    let state = root.path().join("state.json");
    let prompt = "[qol session bridge] Act as the implementation agent for the bounded task below and do not delegate it";
    std::fs::write(
        &state,
        format!(
            r#"{{"createdAt":"t","updatedAt":"t","title":"{prompt}","isCustomTitle":false,"workDir":"/work/proj","lastPrompt":"go"}}"#
        ),
    )
    .unwrap();
    let strategy = strategy(state, "session_abc-123");

    let mut facts = session();
    facts.spawn_identity = Some(crate::SpawnIdentity {
        key: crate::SpawnKey::new("titlecheck-kimi").unwrap(),
        tool: crate::cli::CliToolId::new("kimi").unwrap(),
        surface: crate::SpawnSurface::Tab,
    });

    assert_eq!(
        strategy.describe(&facts).display_name.as_deref(),
        Some("titlecheck-kimi")
    );
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
        spawn_identity: None,
    }
}
