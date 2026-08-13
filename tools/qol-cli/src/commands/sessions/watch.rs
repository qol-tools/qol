use std::ffi::OsString;
use std::io::Write;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use qol_terminal_sessions::{
    ScreenReader, SessionBinding, SessionInventory, TerminalSessionService,
};

use super::bridge::{PendingBridgeStore, PendingRound};
use super::spawn::SpawnLocks;

const POLL_BASE: Duration = Duration::from_secs(10);
const POLL_CAP: Duration = Duration::from_secs(30);
const STALL_AFTER: Duration = Duration::from_secs(15 * 60);
const WATCH_ALL_KEY: &str = "watch-all";

#[derive(Clone, Copy)]
pub(super) struct WatchConfig {
    poll_base: Duration,
    poll_cap: Duration,
    stall_after: Duration,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            poll_base: POLL_BASE,
            poll_cap: POLL_CAP,
            stall_after: STALL_AFTER,
        }
    }
}

struct WatchedRound {
    session: String,
    binding: SessionBinding,
    marker: String,
    reads: u64,
    last_screen: Option<String>,
    last_change: Instant,
    stalled_reported: bool,
}

impl WatchedRound {
    fn new(round: PendingRound) -> Result<Self> {
        let binding = round
            .session
            .parse()
            .map_err(|_| anyhow!("pending checkpoint carries an invalid session token"))?;
        Ok(Self {
            session: round.session,
            binding,
            marker: round.completion_marker,
            reads: 0,
            last_screen: None,
            last_change: Instant::now(),
            stalled_reported: false,
        })
    }
}

pub(super) fn run(args: &[OsString]) -> Result<()> {
    let tokens = args
        .iter()
        .map(|argument| {
            let token = argument
                .to_str()
                .ok_or_else(|| anyhow!("watch tokens must be valid UTF-8"))?
                .to_owned();
            token
                .parse::<SessionBinding>()
                .map_err(|_| anyhow!("invalid session token `{token}`"))?;
            Ok(token)
        })
        .collect::<Result<Vec<_>>>()?;
    watch(
        &TerminalSessionService::system(),
        &PendingBridgeStore::system()?,
        &SpawnLocks::system()?,
        &tokens,
        &mut std::io::stdout(),
        WatchConfig::default(),
    )
}

pub(super) fn watch(
    terminals: &TerminalSessionService,
    pending: &PendingBridgeStore,
    locks: &SpawnLocks,
    tokens: &[String],
    out: &mut dyn Write,
    config: WatchConfig,
) -> Result<()> {
    let watch_lock = if tokens.is_empty() {
        let key = qol_terminal_sessions::SpawnKey::new(WATCH_ALL_KEY)
            .context("watch lock key is invalid")?;
        let guard = locks
            .acquire(&key)
            .context("another qol sessions watch is already running")?;
        Some((key, guard))
    } else {
        None
    };
    let result = (|| {
        let mut watched = load_rounds(pending, tokens)?
            .into_iter()
            .map(WatchedRound::new)
            .collect::<Result<Vec<_>>>()?;
        let mut poll_interval = config.poll_base;
        loop {
            if watched.is_empty() {
                return Ok(());
            }
            reconcile(pending, &mut watched, out)?;
            if watched.is_empty() {
                return Ok(());
            }
            let mut changed = false;
            let mut remaining = Vec::with_capacity(watched.len());
            for mut round in watched {
                let outcome = poll_round(terminals, pending, &mut round, out, config)?;
                changed |= outcome.changed;
                if outcome.keep {
                    remaining.push(round);
                }
            }
            watched = remaining;
            let sleep = if changed {
                config.poll_base
            } else {
                poll_interval
            };
            poll_interval = if changed {
                config.poll_base
            } else {
                next_poll_interval(poll_interval, config.poll_cap)
            };
            std::thread::sleep(sleep);
        }
    })();
    if let Some((key, guard)) = watch_lock {
        drop(guard);
        locks.remove(&key);
    }
    result
}

struct RoundPoll {
    keep: bool,
    changed: bool,
}

fn poll_round(
    terminals: &TerminalSessionService,
    pending: &PendingBridgeStore,
    round: &mut WatchedRound,
    out: &mut dyn Write,
    config: WatchConfig,
) -> Result<RoundPoll> {
    round.reads += 1;
    let screen = if round.reads.is_multiple_of(10) {
        terminals.read_screen(&round.binding)
    } else {
        terminals.read_screen_relaxed(&round.binding)
    };
    let screen = match screen {
        Ok(screen) => screen,
        Err(_) => {
            if session_gone(terminals, &round.binding) {
                emit_gone(out, &round.session)?;
                pending.discard(&round.binding)?;
                qol_runtime::probe!(
                    "CLI_SESSION_WATCH",
                    "event=gone session={} reads={}",
                    round.session,
                    round.reads
                );
                return Ok(RoundPoll {
                    keep: false,
                    changed: true,
                });
            }
            return Ok(RoundPoll {
                keep: true,
                changed: false,
            });
        }
    };
    if screen.contains(&round.marker) {
        emit_completed(out, &round.session, &round.marker)?;
        pending.observe(&round.binding, &round.marker, true)?;
        qol_runtime::probe!(
            "CLI_SESSION_WATCH",
            "event=completed session={} reads={}",
            round.session,
            round.reads
        );
        return Ok(RoundPoll {
            keep: false,
            changed: true,
        });
    }
    let mut changed = false;
    if round.last_screen.as_deref() != Some(screen.as_str()) {
        round.last_screen = Some(screen.clone());
        round.last_change = Instant::now();
        changed = true;
    }
    if !round.stalled_reported && round.last_change.elapsed() >= config.stall_after {
        round.stalled_reported = true;
        emit_stalled(out, &round.session)?;
        qol_runtime::probe!(
            "CLI_SESSION_WATCH",
            "event=stalled session={} reads={}",
            round.session,
            round.reads
        );
        changed = true;
    }
    Ok(RoundPoll {
        keep: true,
        changed,
    })
}

fn reconcile(
    pending: &PendingBridgeStore,
    watched: &mut Vec<WatchedRound>,
    out: &mut dyn Write,
) -> Result<()> {
    let mut remaining = Vec::with_capacity(watched.len());
    for mut round in std::mem::take(watched) {
        match pending.pending_round(&round.binding)? {
            None => {}
            Some(current) => {
                if current.completed {
                    emit_completed(out, &round.session, &current.completion_marker)?;
                    qol_runtime::probe!(
                        "CLI_SESSION_WATCH",
                        "event=completed session={} source=checkpoint",
                        round.session
                    );
                } else {
                    if current.completion_marker != round.marker {
                        round.marker = current.completion_marker;
                        round.reads = 0;
                        round.last_screen = None;
                        round.last_change = Instant::now();
                        round.stalled_reported = false;
                    }
                    remaining.push(round);
                }
            }
        }
    }
    *watched = remaining;
    Ok(())
}

fn load_rounds(pending: &PendingBridgeStore, tokens: &[String]) -> Result<Vec<PendingRound>> {
    if tokens.is_empty() {
        return pending.pending_rounds();
    }
    let mut rounds = Vec::new();
    for token in tokens {
        let binding: SessionBinding = token.parse().context("invalid session token")?;
        if let Some(round) = pending.pending_round(&binding)? {
            rounds.push(round);
        }
    }
    rounds.sort_by(|left, right| left.session.cmp(&right.session));
    Ok(rounds)
}

fn session_gone(terminals: &TerminalSessionService, binding: &SessionBinding) -> bool {
    terminals
        .discover()
        .map(|facts| {
            !facts
                .iter()
                .any(|session| session.id == *binding.session_id())
        })
        .unwrap_or(false)
}

fn next_poll_interval(current: Duration, cap: Duration) -> Duration {
    current.saturating_mul(2).min(cap)
}

fn emit_completed(out: &mut dyn Write, session: &str, marker: &str) -> Result<()> {
    emit(
        out,
        serde_json::json!({ "event": "completed", "session": session, "marker": marker }),
    )
}

fn emit_gone(out: &mut dyn Write, session: &str) -> Result<()> {
    emit(
        out,
        serde_json::json!({ "event": "gone", "session": session }),
    )
}

fn emit_stalled(out: &mut dyn Write, session: &str) -> Result<()> {
    emit(
        out,
        serde_json::json!({ "event": "stalled", "session": session }),
    )
}

fn emit(out: &mut dyn Write, line: serde_json::Value) -> Result<()> {
    writeln!(out, "{line}").context("failed to write watch event")?;
    out.flush().context("failed to flush watch event")
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use qol_terminal_sessions::{
        BackendId, DeliveryMode, SessionCapabilities, SessionFacts, SessionFocus, SessionId,
        TerminalBackend, TerminalError, TerminalSnapshot, TextInput,
    };
    use sha2::{Digest, Sha256};

    use super::super::bridge::PendingBridgeStore;
    use super::super::spawn::SpawnLocks;
    use super::*;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum CallKind {
        Ls,
        Full,
    }

    struct FakeBackend {
        id: BackendId,
        facts: SessionFacts,
        gone: AtomicBool,
        screens: Mutex<VecDeque<String>>,
        last: Mutex<Option<String>>,
        calls: Mutex<Vec<(CallKind, Instant)>>,
    }

    impl FakeBackend {
        fn new(facts: SessionFacts, screens: Vec<String>) -> Arc<Self> {
            Arc::new(Self {
                id: BackendId::new("fake").unwrap(),
                facts,
                gone: AtomicBool::new(false),
                screens: Mutex::new(screens.into()),
                last: Mutex::new(None),
                calls: Mutex::new(Vec::new()),
            })
        }

        fn mark_gone(&self) {
            self.gone.store(true, Ordering::Relaxed);
        }

        fn record(&self, kind: CallKind) {
            self.calls.lock().unwrap().push((kind, Instant::now()));
        }

        fn next_screen(&self) -> String {
            let mut screens = self.screens.lock().unwrap();
            if let Some(screen) = screens.pop_front() {
                *self.last.lock().unwrap() = Some(screen.clone());
                return screen;
            }
            self.last.lock().unwrap().clone().unwrap_or_default()
        }
    }

    impl SessionInventory for FakeBackend {
        fn discover(&self) -> Result<Vec<SessionFacts>, TerminalError> {
            self.record(CallKind::Ls);
            if self.gone.load(Ordering::Relaxed) {
                return Ok(Vec::new());
            }
            Ok(vec![self.facts.clone()])
        }
    }

    impl ScreenReader for FakeBackend {
        fn read_screen(&self, _target: &SessionBinding) -> Result<String, TerminalError> {
            self.record(CallKind::Ls);
            self.record(CallKind::Full);
            if self.gone.load(Ordering::Relaxed) {
                return Err(TerminalError::TargetMissing(_target.clone()));
            }
            Ok(self.next_screen())
        }

        fn read_screen_relaxed(&self, _target: &SessionBinding) -> Result<String, TerminalError> {
            self.record(CallKind::Full);
            if self.gone.load(Ordering::Relaxed) {
                return Err(TerminalError::TargetMissing(_target.clone()));
            }
            Ok(self.next_screen())
        }
    }

    impl SessionFocus for FakeBackend {
        fn focus(&self, _target: &SessionBinding) -> Result<(), TerminalError> {
            Ok(())
        }
    }

    impl TextInput for FakeBackend {
        fn send_text(
            &self,
            _target: &SessionBinding,
            _text: &str,
            _mode: DeliveryMode,
        ) -> Result<(), TerminalError> {
            Ok(())
        }

        fn send_key(&self, _target: &SessionBinding, _key: &str) -> Result<(), TerminalError> {
            Ok(())
        }
    }

    impl TerminalBackend for FakeBackend {
        fn read_screen_from_snapshot(
            &self,
            _snapshot: &TerminalSnapshot,
            _target: &SessionBinding,
        ) -> Result<String, TerminalError> {
            Ok(self.next_screen())
        }

        fn id(&self) -> &BackendId {
            &self.id
        }
    }

    fn facts(native: &str, root_pid: i32) -> SessionFacts {
        SessionFacts {
            id: SessionId::new(BackendId::new("fake").unwrap(), native).unwrap(),
            root_pid,
            cwd: "/work".to_owned(),
            title: "Terminal".to_owned(),
            at_prompt: true,
            reported_cmd: None,
            foreground_basenames: Vec::new(),
            foreground_pids: Vec::new(),
            capabilities: SessionCapabilities::ALL,
            spawn_identity: None,
        }
    }

    fn harness(backend: Arc<FakeBackend>) -> (TerminalSessionService, Arc<FakeBackend>) {
        let terminals = TerminalSessionService::from_backends([
            Arc::clone(&backend) as Arc<dyn TerminalBackend>
        ])
        .unwrap();
        (terminals, backend)
    }

    fn store(root: &tempfile::TempDir) -> PendingBridgeStore {
        PendingBridgeStore::with_dir(root.path().to_path_buf())
    }

    fn locks(root: &tempfile::TempDir) -> SpawnLocks {
        SpawnLocks::with_dir(root.path().join("spawn-locks"))
    }

    fn fast_config(stall_after: Duration) -> WatchConfig {
        WatchConfig {
            poll_base: Duration::from_millis(1),
            poll_cap: Duration::from_millis(4),
            stall_after,
        }
    }

    fn lines(out: &[u8]) -> Vec<serde_json::Value> {
        String::from_utf8_lossy(out)
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn completed_event_fires_and_the_checkpoint_completes() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800")
            .unwrap();
        let backend = FakeBackend::new(
            facts("7", 100),
            vec![
                "idle".to_owned(),
                "idle".to_owned(),
                "done\nQOL_BRIDGE_DONE_round".to_owned(),
            ],
        );
        let (terminals, _) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &pending,
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["session"], "v1:fake:7:100");
        assert_eq!(events[0]["marker"], "QOL_BRIDGE_DONE_round");
        let round = pending.pending_round(&binding).unwrap().unwrap();
        assert!(round.completed);
    }

    #[test]
    fn gone_event_fires_and_the_checkpoint_is_discarded() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800")
            .unwrap();
        let backend = FakeBackend::new(facts("7", 100), Vec::new());
        backend.mark_gone();
        let (terminals, _) = harness(backend);
        let mut out = Vec::new();

        watch(
            &terminals,
            &pending,
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "gone");
        assert_eq!(events[0]["session"], "v1:fake:7:100");
        assert!(pending.pending_round(&binding).unwrap().is_none());
    }

    #[test]
    fn stalled_fires_exactly_once_per_round() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800")
            .unwrap();
        let screens = (0..64)
            .map(|_| "idle".to_owned())
            .chain(["done\nQOL_BRIDGE_DONE_round".to_owned()])
            .collect();
        let backend = FakeBackend::new(facts("7", 100), screens);
        let (terminals, _) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &pending,
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            fast_config(Duration::from_millis(5)),
        )
        .unwrap();

        let events = lines(&out);
        let stalled = events
            .iter()
            .filter(|event| event["event"] == "stalled")
            .collect::<Vec<_>>();
        assert_eq!(stalled.len(), 1, "events: {events:?}");
        assert_eq!(stalled[0]["session"], "v1:fake:7:100");
        let completed = events
            .iter()
            .filter(|event| event["event"] == "completed")
            .collect::<Vec<_>>();
        assert_eq!(completed.len(), 1, "events: {events:?}");
    }

    #[test]
    fn no_change_polls_grow_the_sleep_and_a_change_resets_it() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800")
            .unwrap();
        let screens = [
            "idle".to_owned(),
            "idle".to_owned(),
            "idle".to_owned(),
            "idle".to_owned(),
            "idle".to_owned(),
            "changed".to_owned(),
            "done\nQOL_BRIDGE_DONE_round".to_owned(),
        ]
        .into_iter()
        .collect();
        let backend = FakeBackend::new(facts("7", 100), screens);
        let (terminals, backend) = harness(backend);
        let mut out = Vec::new();

        watch(
            &terminals,
            &pending,
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            WatchConfig {
                poll_base: Duration::from_millis(30),
                poll_cap: Duration::from_millis(120),
                stall_after: Duration::from_secs(3600),
            },
        )
        .unwrap();

        let calls = backend.calls.lock().unwrap();
        let gaps = calls
            .windows(2)
            .map(|pair| pair[1].1 - pair[0].1)
            .collect::<Vec<_>>();
        assert!(gaps.len() >= 6);
        assert!(gaps[1] >= gaps[0] * 2 / 3, "gaps: {gaps:?}");
        assert!(gaps[2] >= gaps[1] * 2 / 3, "gaps: {gaps:?}");
        assert!(gaps[3] >= gaps[2] * 2 / 3, "gaps: {gaps:?}");
        assert!(gaps[4] >= gaps[3] * 2 / 3, "gaps: {gaps:?}");
        assert!(gaps[5] <= gaps[2] * 2 / 3, "gaps: {gaps:?}");
        assert!(gaps[5] <= gaps[0] * 3, "gaps: {gaps:?}");
    }

    #[test]
    fn watch_exits_zero_when_nothing_is_pending() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let (terminals, _) = harness(FakeBackend::new(facts("7", 100), Vec::new()));
        let mut out = Vec::new();

        watch(
            &terminals,
            &pending,
            &locks(&root),
            &[],
            &mut out,
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        assert!(out.is_empty());

        let mut out = Vec::new();
        watch(
            &terminals,
            &pending,
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn watch_cycle_reads_relaxed_with_a_strict_read_every_tenth_poll() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        pending
            .start(&binding, "QOL_BRIDGE_DONE_round", "v1:fake:8:800")
            .unwrap();
        let screens = (0..25)
            .map(|_| "idle".to_owned())
            .chain(["done\nQOL_BRIDGE_DONE_round".to_owned()])
            .collect();
        let backend = FakeBackend::new(facts("7", 100), screens);
        let (terminals, backend) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &pending,
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        let calls = backend.calls.lock().unwrap();
        let full = calls
            .iter()
            .filter(|(kind, _)| *kind == CallKind::Full)
            .count();
        let ls = calls
            .iter()
            .filter(|(kind, _)| *kind == CallKind::Ls)
            .count();
        assert_eq!(full, 26);
        assert_eq!(ls, 2);
        let events = lines(&out);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "completed");
    }

    #[test]
    fn explicit_tokens_limit_the_watched_scope() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let watched_binding: SessionBinding = "v1:fake:7:100".parse().unwrap();
        let other_binding: SessionBinding = "v1:fake:8:200".parse().unwrap();
        pending
            .start(&watched_binding, "QOL_BRIDGE_DONE_watched", "v1:fake:9:900")
            .unwrap();
        pending
            .start(&other_binding, "QOL_BRIDGE_DONE_other", "v1:fake:9:900")
            .unwrap();
        let backend = FakeBackend::new(
            facts("7", 100),
            vec!["done\nQOL_BRIDGE_DONE_watched".to_owned()],
        );
        let (terminals, _) = harness(backend);
        let mut out = Vec::new();
        watch(
            &terminals,
            &pending,
            &locks(&root),
            &["v1:fake:7:100".to_owned()],
            &mut out,
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();

        let events = lines(&out);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["event"], "completed");
        assert_eq!(events[0]["session"], "v1:fake:7:100");
        assert!(
            pending.pending_round(&other_binding).unwrap().is_some(),
            "untouched checkpoints stay pending"
        );
    }

    #[test]
    fn all_rounds_mode_takes_the_watch_lock_so_one_watcher_polls() {
        let root = tempfile::TempDir::new().unwrap();
        let locks = locks(&root);
        let key = qol_terminal_sessions::SpawnKey::new(WATCH_ALL_KEY).unwrap();
        let guard = locks.acquire(&key).unwrap();

        let error = watch(
            &TerminalSessionService::system(),
            &store(&root),
            &locks,
            &[],
            &mut Vec::new(),
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("another qol sessions watch is already running"),
            "{error}"
        );
        drop(guard);
    }

    #[test]
    fn all_rounds_mode_removes_the_watch_lock_on_normal_and_error_exit() {
        let root = tempfile::TempDir::new().unwrap();
        let pending = store(&root);
        let locks = locks(&root);
        let key = qol_terminal_sessions::SpawnKey::new(WATCH_ALL_KEY).unwrap();
        let digest = Sha256::digest(key.as_str().as_bytes());
        let lock_path = root
            .path()
            .join("spawn-locks")
            .join(format!("{digest:x}.lock"));
        let binding: SessionBinding = "v1:fake:7:100".parse().unwrap();

        pending
            .start(&binding, "QOL_BRIDGE_DONE_error", "")
            .unwrap();
        let (terminals, _) = harness(FakeBackend::new(
            facts("7", 100),
            vec!["done\nQOL_BRIDGE_DONE_error".to_owned()],
        ));
        let error = watch(
            &terminals,
            &pending,
            &locks,
            &[],
            &mut FailingWriter,
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("failed to write"), "{error}");
        assert!(
            !lock_path.exists(),
            "an erroring watch must leave no lock file behind"
        );

        pending.discard(&binding).unwrap();
        pending.start(&binding, "QOL_BRIDGE_DONE_done", "").unwrap();
        let (terminals, _) = harness(FakeBackend::new(
            facts("7", 100),
            vec!["done\nQOL_BRIDGE_DONE_done".to_owned()],
        ));
        let mut out = Vec::new();
        watch(
            &terminals,
            &pending,
            &locks,
            &[],
            &mut out,
            fast_config(Duration::from_secs(3600)),
        )
        .unwrap();
        assert!(
            !lock_path.exists(),
            "a completed watch must leave no lock file behind"
        );
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("watch write failed"))
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
}
