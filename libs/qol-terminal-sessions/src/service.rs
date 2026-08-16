use std::collections::BTreeMap;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use crate::{
    BackendId, DeliveryMode, SessionBinding, SessionFacts, SessionId, SessionSpawner, SpawnRequest,
    TerminalError, TerminalSnapshot,
};

pub trait SessionInventory {
    fn discover(&self) -> Result<Vec<SessionFacts>, TerminalError>;
}

pub trait ScreenReader {
    fn read_screen(&self, target: &SessionBinding) -> Result<String, TerminalError>;

    fn read_screen_relaxed(&self, target: &SessionBinding) -> Result<String, TerminalError> {
        self.read_screen(target)
    }
}

pub trait SessionFocus {
    fn focus(&self, target: &SessionBinding) -> Result<(), TerminalError>;
}

pub trait SessionCloser {
    fn close(&self, target: &SessionBinding) -> Result<(), TerminalError>;
}

pub trait TextInput {
    fn send_text(
        &self,
        target: &SessionBinding,
        text: &str,
        mode: DeliveryMode,
    ) -> Result<(), TerminalError>;

    fn send_key(&self, target: &SessionBinding, key: &str) -> Result<(), TerminalError>;
}

pub trait TerminalBackend:
    SessionInventory + ScreenReader + SessionFocus + TextInput + Send + Sync
{
    fn read_screen_from_snapshot(
        &self,
        snapshot: &TerminalSnapshot,
        target: &SessionBinding,
    ) -> Result<String, TerminalError>;

    fn id(&self) -> &BackendId;

    fn current_session_id(&self) -> Option<SessionId> {
        None
    }

    fn spawner(&self) -> Option<&dyn SessionSpawner> {
        None
    }

    fn closer(&self) -> Option<&dyn SessionCloser> {
        None
    }
}

pub struct TerminalSessionService {
    backends: BTreeMap<BackendId, Arc<dyn TerminalBackend>>,
}

impl TerminalSessionService {
    pub fn system() -> Self {
        Self::from_backends([
            Arc::new(crate::kitty::KittyBackend::default()) as Arc<dyn TerminalBackend>
        ])
        .expect("built-in terminal backend ids are unique")
    }

    pub fn from_backends(
        backends: impl IntoIterator<Item = Arc<dyn TerminalBackend>>,
    ) -> Result<Self, TerminalError> {
        let mut registered = BTreeMap::new();
        for backend in backends {
            let id = backend.id().clone();
            if registered.insert(id.clone(), backend).is_some() {
                return Err(TerminalError::DuplicateBackend(id));
            }
        }
        Ok(Self {
            backends: registered,
        })
    }

    pub fn spawn_on(
        &self,
        backend: &BackendId,
        request: &SpawnRequest,
    ) -> Result<SessionId, TerminalError> {
        let backend = self.backend_for_id(backend)?;
        let spawner = backend
            .spawner()
            .ok_or_else(|| TerminalError::SpawnUnsupported {
                backend: backend.id().clone(),
                surface: request.identity.surface,
            })?;
        if !spawner.supports(request.identity.surface) {
            return Err(TerminalError::SpawnUnsupported {
                backend: backend.id().clone(),
                surface: request.identity.surface,
            });
        }
        let spawned = spawner.spawn(request)?;
        if spawned.backend() != backend.id() {
            return Err(TerminalError::SpawnFailed {
                backend: backend.id().clone(),
                message: format!(
                    "spawner returned session `{spawned}` owned by a different backend"
                ),
            });
        }
        Ok(spawned)
    }

    fn backend_for_id(&self, backend: &BackendId) -> Result<&dyn TerminalBackend, TerminalError> {
        self.backends
            .get(backend)
            .map(Arc::as_ref)
            .ok_or_else(|| TerminalError::UnknownBackend(backend.clone()))
    }

    fn backend_for(&self, session_id: &SessionId) -> Result<&dyn TerminalBackend, TerminalError> {
        self.backend_for_id(session_id.backend())
    }
}

impl Default for TerminalSessionService {
    fn default() -> Self {
        Self::system()
    }
}

impl SessionInventory for TerminalSessionService {
    fn discover(&self) -> Result<Vec<SessionFacts>, TerminalError> {
        Ok(self.snapshot()?.sessions().to_vec())
    }
}

impl TerminalSessionService {
    pub fn snapshot(&self) -> Result<TerminalSnapshot, TerminalError> {
        let mut sessions = Vec::new();
        let mut first_error = None;
        for backend in self.backends.values() {
            match backend.discover() {
                Ok(mut discovered) => sessions.append(&mut discovered),
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }
        if sessions.is_empty() {
            if let Some(error) = first_error {
                return Err(error);
            }
        }
        sessions.sort_by(|left, right| left.id.cmp(&right.id));
        let snapshot = TerminalSnapshot::new(sessions);
        qol_runtime::probe!(
            "TERMINAL_SESSIONS",
            "operation=snapshot cache=load age_ms=0 sessions={}",
            snapshot.sessions().len()
        );
        Ok(snapshot)
    }

    pub fn read_screen_from(
        &self,
        snapshot: &TerminalSnapshot,
        target: &SessionBinding,
    ) -> Result<String, TerminalError> {
        snapshot.validate_screen_target(target)?;
        if let Some(screen) = snapshot.cached_screen(target) {
            qol_runtime::probe!(
                "TERMINAL_SESSIONS",
                "operation=read_screen cache=hit age_ms={}",
                snapshot.age_ms()
            );
            return Ok(screen);
        }
        let screen = self
            .backend_for(target.session_id())?
            .read_screen_from_snapshot(snapshot, target)?;
        snapshot.cache_screen(target.clone(), screen.clone());
        qol_runtime::probe!(
            "TERMINAL_SESSIONS",
            "operation=read_screen cache=load age_ms={}",
            snapshot.age_ms()
        );
        Ok(screen)
    }

    pub fn close(&self, target: &SessionBinding) -> Result<(), TerminalError> {
        let backend = self.backend_for(target.session_id())?;
        let closer = backend.closer().ok_or_else(|| TerminalError::Unsupported {
            target: target.session_id().clone(),
            capability: "close",
        })?;
        closer.close(target)
    }

    pub fn is_current(&self, target: &SessionBinding) -> Result<bool, TerminalError> {
        Ok(self
            .backend_for(target.session_id())?
            .current_session_id()
            .as_ref()
            == Some(target.session_id()))
    }
}

impl ScreenReader for TerminalSessionService {
    fn read_screen(&self, target: &SessionBinding) -> Result<String, TerminalError> {
        self.backend_for(target.session_id())?.read_screen(target)
    }

    fn read_screen_relaxed(&self, target: &SessionBinding) -> Result<String, TerminalError> {
        self.backend_for(target.session_id())?
            .read_screen_relaxed(target)
    }
}

impl SessionFocus for TerminalSessionService {
    fn focus(&self, target: &SessionBinding) -> Result<(), TerminalError> {
        self.backend_for(target.session_id())?.focus(target)
    }
}

impl TextInput for TerminalSessionService {
    fn send_text(
        &self,
        target: &SessionBinding,
        text: &str,
        mode: DeliveryMode,
    ) -> Result<(), TerminalError> {
        self.backend_for(target.session_id())?
            .send_text(target, text, mode)
    }

    fn send_key(&self, target: &SessionBinding, key: &str) -> Result<(), TerminalError> {
        self.backend_for(target.session_id())?.send_key(target, key)
    }
}

pub const WAIT_BACKOFF_BASE: Duration = Duration::from_secs(3);
pub const WAIT_BACKOFF_CAP: Duration = Duration::from_secs(15);
const WAIT_SETTLE_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub struct WaitOutcome {
    pub completed: bool,
    pub submitted: bool,
    pub stalled: bool,
    pub screen: String,
    pub reads: u64,
    pub elapsed: Duration,
}

#[derive(Clone, Copy)]
struct WaitBackoff {
    base: Duration,
    cap: Duration,
}

impl Default for WaitBackoff {
    fn default() -> Self {
        Self {
            base: WAIT_BACKOFF_BASE,
            cap: WAIT_BACKOFF_CAP,
        }
    }
}

impl TerminalSessionService {
    #[allow(clippy::too_many_arguments)]
    pub fn wait_for_completion(
        &self,
        binding: &SessionBinding,
        marker: &str,
        timeout: Duration,
        changed: mpsc::Receiver<()>,
        subscribed: bool,
        submitted: bool,
        liveness: &dyn Fn() -> Option<bool>,
        stall_after: Duration,
    ) -> Result<WaitOutcome, TerminalError> {
        self.wait_for_completion_with_backoff(
            binding,
            marker,
            timeout,
            changed,
            subscribed,
            submitted,
            liveness,
            stall_after,
            WaitBackoff::default(),
            &mut |duration| std::thread::sleep(duration),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn wait_for_completion_with_backoff(
        &self,
        binding: &SessionBinding,
        marker: &str,
        timeout: Duration,
        changed: mpsc::Receiver<()>,
        mut subscribed: bool,
        submitted: bool,
        liveness: &dyn Fn() -> Option<bool>,
        stall_after: Duration,
        backoff: WaitBackoff,
        sleep: &mut dyn FnMut(Duration),
    ) -> Result<WaitOutcome, TerminalError> {
        let started = Instant::now();
        let mut previous = None;
        let mut last_change = Instant::now();
        let mut last_probe: Option<Instant> = None;
        let mut reads = 0u64;
        let mut current_backoff = backoff.base;
        loop {
            reads += 1;
            let screen = if reads.is_multiple_of(10) {
                self.read_screen(binding)?
            } else {
                self.read_screen_relaxed(binding)?
            };
            let matched = screen.contains(marker);
            if matched && previous.as_deref() == Some(screen.as_str()) {
                return Ok(WaitOutcome {
                    completed: true,
                    submitted,
                    stalled: false,
                    screen,
                    reads,
                    elapsed: started.elapsed(),
                });
            }
            let screen_changed = previous.as_deref() != Some(screen.as_str());
            if screen_changed {
                last_change = Instant::now();
                current_backoff = backoff.base;
            }
            let grow_backoff = !matched && (previous.is_none() || !screen_changed);
            previous = Some(screen);
            if !matched && stall_if_quiet(&last_change, &mut last_probe, stall_after, liveness) {
                return Ok(WaitOutcome {
                    completed: false,
                    submitted,
                    stalled: true,
                    screen: previous.clone().unwrap_or_default(),
                    reads,
                    elapsed: started.elapsed(),
                });
            }
            let elapsed = started.elapsed();
            if elapsed >= timeout {
                return Ok(WaitOutcome {
                    completed: false,
                    submitted,
                    stalled: false,
                    screen: previous.clone().unwrap_or_default(),
                    reads,
                    elapsed,
                });
            }
            let remaining = timeout.saturating_sub(elapsed);
            let interval = if matched {
                WAIT_SETTLE_INTERVAL
            } else {
                current_backoff
            }
            .min(remaining);
            if grow_backoff {
                current_backoff = next_backoff(current_backoff, backoff.cap);
            }
            if subscribed {
                match changed.recv_timeout(interval) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => subscribed = false,
                }
            } else {
                sleep(interval);
            }
        }
    }
}

fn next_backoff(backoff: Duration, cap: Duration) -> Duration {
    backoff.saturating_mul(2).min(cap)
}

fn stall_if_quiet(
    last_change: &Instant,
    last_probe: &mut Option<Instant>,
    stall_after: Duration,
    liveness: &dyn Fn() -> Option<bool>,
) -> bool {
    if last_change.elapsed() < stall_after {
        return false;
    }
    if last_probe.is_some_and(|probed| probed.elapsed() < stall_after) {
        return false;
    }
    *last_probe = Some(Instant::now());
    let activity = liveness();
    activity == Some(false) || (activity.is_none() && last_change.elapsed() >= stall_after * 4)
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::{Duration, Instant};

    use crate::kitty::KittyBackend;
    use crate::SessionCapabilities;

    use super::*;

    #[test]
    fn duplicate_backend_ids_are_rejected() {
        let backends = [
            Arc::new(KittyBackend::default()) as Arc<dyn TerminalBackend>,
            Arc::new(KittyBackend::default()) as Arc<dyn TerminalBackend>,
        ];

        let error = TerminalSessionService::from_backends(backends)
            .err()
            .expect("duplicate ids must fail");

        assert!(error.to_string().contains("registered twice"));
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum CallKind {
        Ls,
        Full,
    }

    struct FakeBackend {
        id: BackendId,
        facts: SessionFacts,
        screens: Mutex<VecDeque<String>>,
        last: Mutex<Option<String>>,
        calls: Mutex<Vec<(CallKind, Instant)>>,
    }

    impl FakeBackend {
        fn new(screens: Vec<String>) -> Arc<Self> {
            Arc::new(Self {
                id: BackendId::new("fake").unwrap(),
                facts: SessionFacts {
                    id: SessionId::new(BackendId::new("fake").unwrap(), "7").unwrap(),
                    root_pid: 123,
                    cwd: "/work/demo".to_owned(),
                    title: "Demo REPL".to_owned(),
                    at_prompt: true,
                    reported_cmd: Some("agent".to_owned()),
                    foreground_basenames: Vec::new(),
                    foreground_pids: Vec::new(),
                    capabilities: SessionCapabilities::ALL,
                    spawn_identity: None,
                },
                screens: Mutex::new(screens.into()),
                last: Mutex::new(None),
                calls: Mutex::new(Vec::new()),
            })
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
            Ok(vec![self.facts.clone()])
        }
    }

    impl ScreenReader for FakeBackend {
        fn read_screen(&self, _target: &SessionBinding) -> Result<String, TerminalError> {
            self.record(CallKind::Ls);
            self.record(CallKind::Full);
            Ok(self.next_screen())
        }

        fn read_screen_relaxed(&self, _target: &SessionBinding) -> Result<String, TerminalError> {
            self.record(CallKind::Full);
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

    struct StopSignal(Arc<AtomicBool>);

    impl Drop for StopSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    fn ticker() -> (mpsc::Receiver<()>, StopSignal) {
        let (tx, rx) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let tick_stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            while !tick_stop.load(Ordering::Relaxed) {
                let _ = tx.try_send(());
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        (rx, StopSignal(stop))
    }

    fn binding() -> SessionBinding {
        SessionBinding::new(
            SessionId::new(BackendId::new("fake").unwrap(), "7").unwrap(),
            123,
        )
        .unwrap()
    }

    fn service(backend: Arc<FakeBackend>) -> TerminalSessionService {
        TerminalSessionService::from_backends([backend as Arc<dyn TerminalBackend>]).unwrap()
    }

    fn count(calls: &[(CallKind, Instant)], kind: CallKind) -> usize {
        calls.iter().filter(|(call, _)| *call == kind).count()
    }

    #[test]
    fn every_tenth_poll_is_a_strict_full_read() {
        let screens = vec!["idle".to_owned(); 25]
            .into_iter()
            .chain(["done\nQOL_BRIDGE_DONE_a".to_owned()])
            .collect();
        let backend = FakeBackend::new(screens);
        let terminals = service(backend.clone());
        let (rx, _stop) = ticker();

        let outcome = terminals
            .wait_for_completion(
                &binding(),
                "QOL_BRIDGE_DONE_a",
                Duration::from_secs(60),
                rx,
                true,
                true,
                &|| None,
                Duration::from_secs(3600),
            )
            .unwrap();

        assert!(outcome.completed);
        assert_eq!(outcome.reads, 27);
        let calls = backend.calls.lock().unwrap();
        assert_eq!(count(&calls, CallKind::Ls), 2);
        assert_eq!(count(&calls, CallKind::Full), 27);
    }

    #[test]
    fn settle_requires_the_marker_on_two_stable_reads() {
        let screens = vec!["idle".to_owned(), "done\nQOL_BRIDGE_DONE_a".to_owned()];
        let backend = FakeBackend::new(screens);
        let terminals = service(backend.clone());
        let (rx, _stop) = ticker();

        let outcome = terminals
            .wait_for_completion(
                &binding(),
                "QOL_BRIDGE_DONE_a",
                Duration::from_secs(60),
                rx,
                true,
                true,
                &|| None,
                Duration::from_secs(3600),
            )
            .unwrap();

        assert!(outcome.completed);
        assert_eq!(outcome.reads, 3);
        let calls = backend.calls.lock().unwrap();
        assert_eq!(count(&calls, CallKind::Full), 3);
        assert_eq!(count(&calls, CallKind::Ls), 0);
    }

    #[test]
    fn backoff_doubles_from_base_and_caps() {
        let cap = Duration::from_secs(15);
        assert_eq!(
            next_backoff(Duration::from_secs(3), cap),
            Duration::from_secs(6)
        );
        assert_eq!(
            next_backoff(Duration::from_secs(6), cap),
            Duration::from_secs(12)
        );
        assert_eq!(
            next_backoff(Duration::from_secs(12), cap),
            Duration::from_secs(15)
        );
        assert_eq!(
            next_backoff(Duration::from_secs(15), cap),
            Duration::from_secs(15)
        );
    }

    #[test]
    fn no_change_polls_grow_the_sleep_up_to_the_cap() {
        let mut screens = vec!["idle".to_owned(); 8];
        screens.push("done\nQOL_BRIDGE_DONE_never".to_owned());
        let backend = FakeBackend::new(screens);
        let terminals = service(backend.clone());
        let (tx, rx) = mpsc::sync_channel::<()>(1);
        drop(tx);
        let mut requested = Vec::new();

        let outcome = terminals
            .wait_for_completion_with_backoff(
                &binding(),
                "QOL_BRIDGE_DONE_never",
                Duration::from_secs(30),
                rx,
                false,
                true,
                &|| None,
                Duration::from_secs(3600),
                WaitBackoff {
                    base: Duration::from_millis(30),
                    cap: Duration::from_millis(120),
                },
                &mut |duration| {
                    requested.push(duration);
                    std::thread::sleep(duration);
                },
            )
            .unwrap();

        assert!(outcome.completed);
        assert_eq!(
            outcome.reads, 10,
            "completion needs two stable matching reads"
        );
        assert_eq!(
            &requested[..5],
            &[
                Duration::from_millis(30),
                Duration::from_millis(60),
                Duration::from_millis(120),
                Duration::from_millis(120),
                Duration::from_millis(120),
            ],
            "the sleep must double from the base until it caps"
        );
    }

    #[test]
    fn a_change_resets_the_backoff_to_base() {
        let screens = vec![
            "idle".to_owned(),
            "idle".to_owned(),
            "idle".to_owned(),
            "done\nQOL_BRIDGE_DONE_a".to_owned(),
            "idle2".to_owned(),
        ];
        let backend = FakeBackend::new(screens);
        let terminals = service(backend.clone());
        let (tx, rx) = mpsc::sync_channel::<()>(1);
        drop(tx);
        let mut requested = Vec::new();

        let outcome = terminals
            .wait_for_completion_with_backoff(
                &binding(),
                "QOL_BRIDGE_DONE_a",
                Duration::from_secs(2),
                rx,
                false,
                true,
                &|| None,
                Duration::from_secs(3600),
                WaitBackoff {
                    base: Duration::from_millis(30),
                    cap: Duration::from_millis(120),
                },
                &mut |duration| {
                    requested.push(duration);
                    std::thread::sleep(duration);
                },
            )
            .unwrap();

        assert!(!outcome.completed);
        assert_eq!(
            &requested[..7],
            &[
                Duration::from_millis(30),
                Duration::from_millis(60),
                Duration::from_millis(120),
                WAIT_SETTLE_INTERVAL,
                Duration::from_millis(30),
                Duration::from_millis(30),
                Duration::from_millis(60),
            ],
            "a changed screen must drop the sleep back to the base"
        );
    }
}
