use super::{platform, policy, trace, Observation, PolicyIntent};
use crate::surfaces::show_plugin_notification;
use qol_runtime::protocol::NotificationLevel;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

type Probe = Arc<dyn Fn() -> (Observation, PolicyIntent) + Send + Sync>;

struct Generation {
    in_flight: AtomicUsize,
    quiesced: std::sync::Mutex<()>,
    condvar: std::sync::Condvar,
}

impl Generation {
    fn new() -> Self {
        Self {
            in_flight: AtomicUsize::new(0),
            quiesced: std::sync::Mutex::new(()),
            condvar: std::sync::Condvar::new(),
        }
    }

    fn register(self: &Arc<Self>) -> InFlightGuard {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        InFlightGuard {
            generation: self.clone(),
        }
    }

    fn signal_quiesced(&self) {
        if self.in_flight.fetch_sub(1, Ordering::SeqCst) == 1 {
            let _guard = self
                .quiesced
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            self.condvar.notify_all();
        }
    }

    fn wait_quiesced(&self, timeout: Option<std::time::Duration>) -> bool {
        let mut guard = self
            .quiesced
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let deadline = timeout.map(|timeout| std::time::Instant::now() + timeout);
        loop {
            if self.in_flight.load(Ordering::SeqCst) == 0 {
                return true;
            }
            let Some(deadline) = deadline else {
                guard = self
                    .condvar
                    .wait(guard)
                    .unwrap_or_else(|poison| poison.into_inner());
                continue;
            };
            let now = std::time::Instant::now();
            if now >= deadline {
                return false;
            }
            let (next, _) = self
                .condvar
                .wait_timeout(guard, deadline - now)
                .unwrap_or_else(|poison| poison.into_inner());
            guard = next;
        }
    }
}

struct InFlightGuard {
    generation: Arc<Generation>,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.generation.signal_quiesced();
    }
}

#[cfg(test)]
const STOP_BACKSTOP: std::time::Duration = std::time::Duration::from_millis(300);

#[cfg(not(test))]
const STOP_BACKSTOP: std::time::Duration = std::time::Duration::from_secs(3);

struct Gate {
    flag: std::sync::Mutex<bool>,
    condvar: std::sync::Condvar,
}

impl Gate {
    fn new() -> Self {
        Self {
            flag: std::sync::Mutex::new(false),
            condvar: std::sync::Condvar::new(),
        }
    }

    fn signal(&self) {
        *self
            .flag
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = true;
        self.condvar.notify_all();
    }

    fn wait_indefinite(&self) {
        let mut flag = self
            .flag
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        while !*flag {
            flag = self
                .condvar
                .wait(flag)
                .unwrap_or_else(|poison| poison.into_inner());
        }
    }
}

static NEXT_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct WatcherControl {
    generation_id: u64,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    generation: Arc<Generation>,
}

enum WatchState {
    Idle,
    Running(WatcherControl),
    Stopping(WatcherControl),
}

static WATCH_STATE: OnceLock<(Mutex<WatchState>, std::sync::Condvar)> = OnceLock::new();

fn watch_state() -> &'static (Mutex<WatchState>, std::sync::Condvar) {
    WATCH_STATE.get_or_init(|| (Mutex::new(WatchState::Idle), std::sync::Condvar::new()))
}

pub(super) fn stop_watch() {
    let (mutex, condvar) = watch_state();
    let mut guard = mutex.lock().unwrap_or_else(|poison| poison.into_inner());
    let control = match &mut *guard {
        WatchState::Idle => return,
        WatchState::Stopping(control) => control.clone(),
        WatchState::Running(control) => {
            let control = control.clone();
            *guard = WatchState::Stopping(control.clone());
            drop(guard);
            condvar.notify_all();
            let _ = control.shutdown_tx.send(true);
            return finish_stop(control);
        }
    };
    drop(guard);
    finish_stop(control);
}

fn finish_stop(control: WatcherControl) {
    let deadline = std::time::Instant::now() + STOP_BACKSTOP;
    control.generation.wait_quiesced(Some(STOP_BACKSTOP));
    wait_until_idle_published(deadline);
}

fn wait_until_idle_published(deadline: std::time::Instant) -> bool {
    let (mutex, condvar) = watch_state();
    let mut guard = mutex.lock().unwrap_or_else(|poison| poison.into_inner());
    loop {
        if matches!(&*guard, WatchState::Idle) {
            return true;
        }
        let now = std::time::Instant::now();
        if now >= deadline {
            return false;
        }
        let (next, _) = condvar
            .wait_timeout(guard, deadline - now)
            .unwrap_or_else(|poison| poison.into_inner());
        guard = next;
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Latch {
    Idle,
    Pending { loaded: String, on_disk: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LatchAction {
    Preserved,
    Cleared,
    Sent,
    Deduped,
}

impl Latch {
    pub(crate) fn next(latch: Latch, observation: &Observation) -> (Latch, LatchAction) {
        match observation {
            Observation::Unsupported
            | Observation::LoadedUnavailable
            | Observation::OnDiskUnavailable { .. } => (latch, LatchAction::Preserved),
            Observation::NotLoaded | Observation::Matched { .. } => {
                (Latch::Idle, LatchAction::Cleared)
            }
            Observation::Mismatch { loaded, on_disk } => match &latch {
                Latch::Pending {
                    loaded: prev_loaded,
                    on_disk: prev_on_disk,
                } if prev_loaded == loaded && prev_on_disk == on_disk => {
                    (latch, LatchAction::Deduped)
                }
                _ => (
                    Latch::Pending {
                        loaded: loaded.clone(),
                        on_disk: on_disk.clone(),
                    },
                    LatchAction::Sent,
                ),
            },
        }
    }
}

pub(super) fn spawn() {
    if !platform::watch_supported() {
        trace::observe(&Observation::Unsupported);
        return;
    }
    let observe: Arc<dyn Fn() -> Observation + Send + Sync> = Arc::new(super::observe);
    let intent: Arc<dyn Fn() -> PolicyIntent + Send + Sync> = Arc::new(super::policy_intent);
    let probe: Probe = Arc::new(move || (observe(), intent()));
    let roots = vec![
        qol_watch::WatchRoot::deep("/lib/modules"),
        qol_watch::WatchRoot::shallow("/proc/driver"),
    ];
    spawn_watcher(probe, roots);
}

fn spawn_watcher(probe: Probe, roots: Vec<qol_watch::WatchRoot>) {
    let (mutex, condvar) = watch_state();
    let mut guard = mutex.lock().unwrap_or_else(|poison| poison.into_inner());
    loop {
        match &*guard {
            WatchState::Stopping(_) => {
                guard = condvar
                    .wait(guard)
                    .unwrap_or_else(|poison| poison.into_inner());
            }
            WatchState::Running(_) => return,
            WatchState::Idle => break,
        }
    }
    let generation_id = NEXT_GENERATION.fetch_add(1, Ordering::SeqCst);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let generation = Arc::new(Generation::new());
    let publish = Arc::new(Gate::new());
    let control = WatcherControl {
        generation_id,
        shutdown_tx: shutdown_tx.clone(),
        generation: generation.clone(),
    };
    let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let supervisor_publish = publish.clone();
    let supervisor_generation = generation.clone();
    let supervisor = std::thread::Builder::new()
        .name("qol-gpu-driver-sync-supervisor".to_string())
        .spawn(move || {
            supervisor_entry(
                probe,
                roots,
                shutdown_rx,
                ready_tx,
                supervisor_publish,
                supervisor_generation,
                generation_id,
            )
        });
    if let Err(error) = supervisor {
        log::warn!(
            "gpu_driver_sync: failed to spawn the supervisor thread ({error}); the lifecycle stays idle"
        );
        return;
    }
    match ready_rx.recv() {
        Ok(Ok(())) => {
            *guard = WatchState::Running(control);
        }
        Ok(Err(detail)) => {
            log::warn!(
                "gpu_driver_sync: watcher startup failed ({detail}); the lifecycle stays idle"
            );
            let _ = shutdown_tx.send(true);
        }
        Err(_) => {
            log::warn!(
                "gpu_driver_sync: watcher startup disconnected without a readiness result; the lifecycle stays idle"
            );
            let _ = shutdown_tx.send(true);
        }
    }
    publish.signal();
}

#[cfg(test)]
fn injected_watcher_spawn_failure() -> Option<std::io::Error> {
    std::env::var_os("QOL_WATCH_WORKER_SPAWN_FAILURE")
        .map(|_| std::io::Error::other("injected watcher thread spawn failure"))
}

#[cfg(not(test))]
fn injected_watcher_spawn_failure() -> Option<std::io::Error> {
    None
}

fn supervisor_entry(
    probe: Probe,
    roots: Vec<qol_watch::WatchRoot>,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ready_tx: std::sync::mpsc::Sender<Result<(), String>>,
    publish: Arc<Gate>,
    generation: Arc<Generation>,
    generation_id: u64,
) {
    let (notice_tx, notice_rx) = tokio::sync::mpsc::channel::<qol_watch::WatchNotice>(1);
    let ready_on_spawn_failure = ready_tx.clone();
    let watcher = if let Some(error) = injected_watcher_spawn_failure() {
        Err(error)
    } else {
        std::thread::Builder::new()
            .name("qol-gpu-driver-sync".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        log::warn!(
                            "gpu_driver_sync: failed to build the watcher runtime ({error}); the watcher cannot start"
                        );
                        let _ = ready_tx.send(Err(format!("{error}")));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                #[cfg(test)]
                if std::env::var_os("QOL_WATCH_WORKER_PANIC").is_some() {
                    panic!("injected watcher worker panic");
                }
                runtime.block_on(run_watcher(
                    notice_tx,
                    notice_rx,
                    roots,
                    shutdown_rx,
                    probe,
                    generation,
                ));
            })
    };
    if let Err(error) = &watcher {
        log::warn!(
            "gpu_driver_sync: failed to spawn the watcher thread ({error}); the lifecycle stays idle"
        );
        let _ = ready_on_spawn_failure.send(Err(format!("{error}")));
    }
    publish.wait_indefinite();
    if let Ok(join) = watcher {
        if join.join().is_err() {
            log::warn!("gpu_driver_sync: the watcher thread panicked");
        }
    }
    let (mutex, condvar) = watch_state();
    let mut guard = mutex.lock().unwrap_or_else(|poison| poison.into_inner());
    match &mut *guard {
        WatchState::Running(control) | WatchState::Stopping(control)
            if control.generation_id == generation_id =>
        {
            *guard = WatchState::Idle;
        }
        _ => {}
    }
    drop(guard);
    condvar.notify_all();
}

async fn run_watcher(
    notice_tx: tokio::sync::mpsc::Sender<qol_watch::WatchNotice>,
    notice_rx: tokio::sync::mpsc::Receiver<qol_watch::WatchNotice>,
    roots: Vec<qol_watch::WatchRoot>,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
    probe: Probe,
    generation: Arc<Generation>,
) {
    match qol_watch::watch(&roots, move |notice| {
        let _ = notice_tx.try_send(notice);
    }) {
        Ok(watch) => run_latch_loop(notice_rx, watch, shutdown_rx, probe, generation).await,
        Err(error) => {
            log::warn!(
                "gpu_driver_sync: kernel-module event watch unavailable ({error}); only the initial observation runs"
            );
            run_initial_only(probe, shutdown_rx, generation).await
        }
    }
}

async fn run_initial_only(
    probe: Probe,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    generation: Arc<Generation>,
) {
    tokio::select! {
        _ = shutdown_rx.changed() => {}
        _ = async {
            let (observation, intent) = probe_blocking(generation.clone(), probe).await;
            trace::observe(&observation);
            let (_, action) = Latch::next(Latch::Idle, &observation);
            if action == LatchAction::Sent {
                send_notification(&observation, &intent);
            }
        } => {}
    }
    while !*shutdown_rx.borrow_and_update() && shutdown_rx.changed().await.is_ok() {}
    quiesce_after_shutdown(&generation);
}

async fn probe_blocking(generation: Arc<Generation>, probe: Probe) -> (Observation, PolicyIntent) {
    let ticket = generation.register();
    tokio::task::spawn_blocking(move || {
        let _ticket = ticket;
        probe()
    })
    .await
    .unwrap_or((Observation::LoadedUnavailable, PolicyIntent::Unavailable))
}

fn quiesce_after_shutdown(generation: &Generation) {
    generation.wait_quiesced(None);
}

async fn run_latch_loop(
    mut notice_rx: tokio::sync::mpsc::Receiver<qol_watch::WatchNotice>,
    _watch: qol_watch::Watch,
    mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    probe: Probe,
    generation: Arc<Generation>,
) {
    let mut latch = Latch::Idle;
    loop {
        let (observation, intent) = probe_blocking(generation.clone(), probe.clone()).await;
        trace::observe(&observation);
        let (next_latch, action) = Latch::next(latch, &observation);
        latch = next_latch;
        match action {
            LatchAction::Sent => send_notification(&observation, &intent),
            LatchAction::Deduped => trace::notify("deduped", None, None, None),
            LatchAction::Preserved => trace::notify("preserved", None, None, None),
            LatchAction::Cleared => trace::notify("cleared", None, None, None),
        }
        let shutdown = tokio::select! {
            _ = shutdown_rx.changed() => true,
            notice = notice_rx.recv() => notice.is_none(),
        };
        if shutdown {
            break;
        }
        while notice_rx.try_recv().is_ok() {}
    }
    quiesce_after_shutdown(&generation);
}

fn send_notification(observation: &Observation, intent: &PolicyIntent) {
    if let Observation::Mismatch { loaded, on_disk } = observation {
        log::warn!(
            "gpu_driver_sync: kernel runs NVIDIA {loaded} but on-disk module is {on_disk} (policy={})",
            intent.as_str()
        );
        show_plugin_notification(
            "QoL Tray",
            &policy::notification_text(loaded, on_disk, intent),
            NotificationLevel::Error,
            None,
        );
        trace::notify("sent", Some(loaded), Some(on_disk), Some(intent.as_str()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn generation() -> Arc<Generation> {
        Arc::new(Generation::new())
    }

    fn counting_probe(counter: Arc<AtomicUsize>) -> Probe {
        Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            (Observation::NotLoaded, PolicyIntent::None)
        })
    }

    fn instant_probe(probe_tx: std::sync::mpsc::Sender<()>) -> Probe {
        Arc::new(move || {
            let _ = probe_tx.send(());
            (Observation::NotLoaded, PolicyIntent::None)
        })
    }

    #[test]
    fn the_initial_observation_runs_even_when_only_initial_observation_is_possible() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let (probe_tx, probe_rx) = std::sync::mpsc::channel::<()>();
            let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            let gen = generation();
            let handle = tokio::spawn(run_initial_only(
                instant_probe(probe_tx),
                shutdown_rx,
                gen.clone(),
            ));
            probe_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("the initial observation must run even without a live watch");
            let _ = shutdown_tx.send(true);
            tokio::time::timeout(Duration::from_secs(5), handle)
                .await
                .expect("the initial-only task must terminate on shutdown")
                .unwrap();
            assert!(
                gen.wait_quiesced(Some(Duration::from_secs(5))),
                "the initial-only generation must quiesce on shutdown"
            );
        });
    }

    #[test]
    fn initial_only_stays_alive_until_shutdown_after_its_one_observation() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let (probe_tx, probe_rx) = std::sync::mpsc::channel::<()>();
            let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            let gen = generation();
            let handle = tokio::spawn(run_initial_only(
                instant_probe(probe_tx),
                shutdown_rx,
                gen.clone(),
            ));
            probe_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("the initial observation must complete");
            assert!(
                !handle.is_finished(),
                "the initial-only task must stay alive after its observation"
            );
            let _ = _shutdown_tx.send(true);
            tokio::time::timeout(Duration::from_secs(5), handle)
                .await
                .expect("shutdown must release the initial-only task")
                .unwrap();
            assert!(gen.wait_quiesced(Some(Duration::from_secs(5))));
        });
    }

    #[test]
    fn a_dropped_shutdown_sender_cannot_busy_spin_the_initial_only_task() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let (probe_tx, probe_rx) = std::sync::mpsc::channel::<()>();
            let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            let gen = generation();
            let handle = tokio::spawn(run_initial_only(
                instant_probe(probe_tx),
                shutdown_rx,
                gen.clone(),
            ));
            probe_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("the initial observation must run");
            drop(shutdown_tx);
            tokio::time::timeout(Duration::from_secs(5), handle)
                .await
                .expect(
                    "a dropped shutdown sender must release the initial-only task instead of busy-spinning",
                )
                .unwrap();
            assert!(
                gen.wait_quiesced(Some(Duration::from_secs(5))),
                "the generation must quiesce after the dropped sender releases the task"
            );
        });
    }

    #[test]
    fn shutdown_signals_terminate_the_watch_task() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let dir =
                std::env::temp_dir().join(format!("qol-watch-shutdown-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let (notice_tx, notice_rx) = tokio::sync::mpsc::channel(64);
            let watch = qol_watch::watch(&[qol_watch::WatchRoot::shallow(&dir)], move |notice| {
                let _ = notice_tx.try_send(notice);
            })
            .unwrap();
            let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            let counter = Arc::new(AtomicUsize::new(0));
            let gen = generation();
            let handle = tokio::spawn(run_latch_loop(
                notice_rx,
                watch,
                shutdown_rx,
                counting_probe(counter.clone()),
                gen.clone(),
            ));
            let _ = shutdown_tx.send(true);
            tokio::time::timeout(Duration::from_secs(5), handle)
                .await
                .expect("the watch task must terminate on shutdown")
                .unwrap();
            assert!(
                gen.wait_quiesced(Some(Duration::from_secs(5))),
                "the watcher generation must quiesce on shutdown"
            );
            std::fs::remove_dir_all(&dir).ok();
        });
    }

    #[test]
    fn a_real_filesystem_event_triggers_reobservation() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let dir = std::env::temp_dir().join(format!("qol-watch-event-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let (notice_tx, notice_rx) = tokio::sync::mpsc::channel(64);
            let watch = qol_watch::watch(&[qol_watch::WatchRoot::shallow(&dir)], move |notice| {
                let _ = notice_tx.try_send(notice);
            })
            .unwrap();
            let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            let (probe_tx, probe_rx) = std::sync::mpsc::channel::<()>();
            let counter = Arc::new(AtomicUsize::new(0));
            let probe_counter = counter.clone();
            let probe: Probe = Arc::new(move || {
                probe_counter.fetch_add(1, Ordering::SeqCst);
                let _ = probe_tx.send(());
                (Observation::NotLoaded, PolicyIntent::None)
            });
            let handle = tokio::spawn(run_latch_loop(
                notice_rx,
                watch,
                shutdown_rx,
                probe,
                generation(),
            ));
            probe_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("the initial observation must run");
            std::fs::write(dir.join("module.ko"), b"payload").unwrap();
            let second = counter.load(Ordering::SeqCst);
            probe_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("a real filesystem event must drive a re-observation");
            assert!(
                counter.load(Ordering::SeqCst) > second,
                "the event must trigger a fresh probe"
            );
            let _ = shutdown_tx.send(true);
            handle.abort();
            std::fs::remove_dir_all(&dir).ok();
        });
    }

    #[test]
    fn a_panicking_probe_falls_back_and_quiesces_on_shutdown() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let (panicked_tx, panicked_rx) = std::sync::mpsc::channel::<()>();
            let probe: Probe = Arc::new(move || {
                let _ = panicked_tx.send(());
                panic!("injected probe panic");
            });
            let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            let gen = generation();
            let handle = tokio::spawn(run_initial_only(probe, shutdown_rx, gen.clone()));
            panicked_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("the panicking probe must start");
            let _ = shutdown_tx.send(true);
            tokio::time::timeout(Duration::from_secs(5), handle)
                .await
                .expect("the initial-only task must survive a panicked probe and join on shutdown")
                .unwrap();
            assert!(
                gen.wait_quiesced(Some(Duration::from_secs(5))),
                "a panicked probe must still release its in-flight registration"
            );
            assert_eq!(gen.in_flight.load(Ordering::SeqCst), 0);
        });
    }

    #[test]
    fn a_shutdown_while_a_probe_is_in_flight_waits_for_that_probe_before_quiescing() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
            let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
            let gate = Arc::new(Mutex::new(gate_rx));
            let probe_gate = gate.clone();
            let probe: Probe = Arc::new(move || {
                let _ = started_tx.send(());
                probe_gate.lock().unwrap().recv().unwrap();
                (Observation::NotLoaded, PolicyIntent::None)
            });
            let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            let gen = generation();
            let handle = tokio::spawn(run_initial_only(probe, shutdown_rx, gen.clone()));
            started_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("the gated probe must start and block");
            assert_eq!(gen.in_flight.load(Ordering::SeqCst), 1);
            let _ = shutdown_tx.send(true);
            assert!(
                !handle.is_finished(),
                "shutdown must not complete the task while the in-flight probe is blocked"
            );
            let _ = gate_tx.send(());
            tokio::time::timeout(Duration::from_secs(5), handle)
                .await
                .expect("releasing the probe must let the task quiesce and join")
                .unwrap();
            assert!(gen.wait_quiesced(Some(Duration::from_secs(5))));
            assert_eq!(gen.in_flight.load(Ordering::SeqCst), 0);
        });
    }

    #[test]
    fn probe_quiescence_signals_when_in_flight_probes_complete() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let gen = generation();
            let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
            let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
            let gate_for_probe = Arc::new(Mutex::new(gate_rx));
            let probe_gate = gate_for_probe.clone();
            let probe: Probe = Arc::new(move || {
                let _ = started_tx.send(());
                probe_gate.lock().unwrap().recv().unwrap();
                (Observation::NotLoaded, PolicyIntent::None)
            });
            struct ReleaseGate {
                tx: std::sync::mpsc::Sender<()>,
            }
            impl Drop for ReleaseGate {
                fn drop(&mut self) {
                    let _ = self.tx.send(());
                }
            }
            let _release = ReleaseGate {
                tx: gate_tx.clone(),
            };
            let probe_future = tokio::spawn(probe_blocking(gen.clone(), probe));
            started_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("the probe must register and start");
            assert_eq!(
                gen.in_flight.load(Ordering::SeqCst),
                1,
                "the quiescence registration must precede the blocking work"
            );
            let (waiter_tx, mut waiter_rx) = tokio::sync::oneshot::channel();
            let waiter_gen = gen.clone();
            let waiter = tokio::spawn(async move {
                quiesce_after_shutdown(&waiter_gen);
                let _ = waiter_tx.send(());
            });
            assert!(
                waiter_rx.try_recv().is_err(),
                "shutdown must not observe zero while the probe is blocked"
            );
            let _ = gate_tx.send(());
            tokio::time::timeout(Duration::from_secs(5), &mut waiter_rx)
                .await
                .expect("quiescence must be signalled once the probe completes")
                .unwrap();
            probe_future.await.unwrap();
            waiter.await.unwrap();
            assert_eq!(gen.in_flight.load(Ordering::SeqCst), 0);
        });
    }
}

#[cfg(test)]
fn serialized_watch_tests() -> std::sync::MutexGuard<'static, ()> {
    static GUARD: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
    GUARD
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use std::time::Duration;

    fn reset_state() {
        let (mutex, _) = watch_state();
        *mutex.lock().unwrap_or_else(|poison| poison.into_inner()) = WatchState::Idle;
    }

    fn running_watcher_count() -> usize {
        let (mutex, _) = watch_state();
        match &*mutex.lock().unwrap_or_else(|poison| poison.into_inner()) {
            WatchState::Running(_) => 1,
            WatchState::Idle | WatchState::Stopping(_) => 0,
        }
    }

    fn wait_until_idle(timeout: Duration) -> bool {
        let (mutex, condvar) = watch_state();
        let mut guard = mutex.lock().unwrap_or_else(|poison| poison.into_inner());
        let deadline = std::time::Instant::now() + timeout;
        while !matches!(&*guard, WatchState::Idle) {
            let now = std::time::Instant::now();
            if now >= deadline {
                return false;
            }
            let (next, _) = condvar
                .wait_timeout(guard, deadline - now)
                .unwrap_or_else(|poison| poison.into_inner());
            guard = next;
        }
        true
    }

    fn state_is_stopping() -> bool {
        let (mutex, _) = watch_state();
        matches!(
            &*mutex.lock().unwrap_or_else(|poison| poison.into_inner()),
            WatchState::Stopping(_)
        )
    }

    fn concurrency_watermark_probe(concurrent: Arc<AtomicUsize>, peak: Arc<AtomicUsize>) -> Probe {
        Arc::new(move || {
            let now = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            concurrent.fetch_sub(1, Ordering::SeqCst);
            (Observation::NotLoaded, PolicyIntent::None)
        })
    }

    #[cfg(target_os = "linux")]
    fn hanging_modinfo_script(dir: &std::path::Path) -> std::path::PathBuf {
        let script = dir.join("modinfo");
        std::fs::write(&script, "#!/bin/sh\nsleep 30\n").unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o755);
        std::fs::set_permissions(&script, permissions).unwrap();
        script
    }

    #[cfg(target_os = "linux")]
    fn hanging_probe(
        script: std::path::PathBuf,
        started: std::sync::mpsc::Sender<()>,
        concurrent: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
    ) -> Probe {
        Arc::new(move || {
            let _ = started.send(());
            let now = concurrent.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(now, Ordering::SeqCst);
            let _ = super::platform::bounded_modinfo_version(&script);
            concurrent.fetch_sub(1, Ordering::SeqCst);
            (
                Observation::OnDiskUnavailable {
                    loaded: "580.159.02".to_string(),
                },
                PolicyIntent::None,
            )
        })
    }

    fn gated_probe(
        started: std::sync::mpsc::Sender<()>,
        gate: Arc<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>,
    ) -> Probe {
        Arc::new(move || {
            let _ = started.send(());
            gate.lock().unwrap().recv().unwrap();
            (Observation::NotLoaded, PolicyIntent::None)
        })
    }

    #[test]
    fn duplicate_spawn_stop_and_restart_share_one_serialized_lifecycle() {
        let _serial = serialized_watch_tests();
        reset_state();
        let dir = std::env::temp_dir().join(format!("qol-watch-lifecycle-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let concurrent = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        spawn_watcher(
            concurrency_watermark_probe(concurrent.clone(), peak.clone()),
            vec![qol_watch::WatchRoot::shallow(&dir)],
        );
        spawn_watcher(
            concurrency_watermark_probe(concurrent.clone(), peak.clone()),
            vec![qol_watch::WatchRoot::shallow(&dir)],
        );
        assert_eq!(running_watcher_count(), 1, "exactly one watcher must exist");
        stop_watch();
        assert_eq!(
            running_watcher_count(),
            0,
            "a successful bounded stop must imply the exact join and the published Idle"
        );
        assert_eq!(peak.load(Ordering::SeqCst), 1, "probes must never overlap");
        reset_state();
        spawn_watcher(
            concurrency_watermark_probe(concurrent.clone(), peak.clone()),
            vec![qol_watch::WatchRoot::shallow(&dir)],
        );
        assert_eq!(
            running_watcher_count(),
            1,
            "a restart after stop must start exactly one fresh watcher"
        );
        stop_watch();
        assert!(wait_until_idle(Duration::from_secs(5)));
        reset_state();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_panicking_probe_quiesces_and_stop_returns() {
        let _serial = serialized_watch_tests();
        reset_state();
        let dir = std::env::temp_dir().join(format!("qol-watch-panic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (panicked_tx, panicked_rx) = std::sync::mpsc::channel::<()>();
        let has_panicked = Arc::new(AtomicUsize::new(0));
        let panic_flag = has_panicked.clone();
        let probe: Probe = Arc::new(move || {
            if panic_flag.fetch_add(1, Ordering::SeqCst) == 0 {
                let _ = panicked_tx.send(());
                panic!("the gated probe panics");
            }
            (Observation::NotLoaded, PolicyIntent::None)
        });
        spawn_watcher(probe, vec![qol_watch::WatchRoot::shallow(&dir)]);
        panicked_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the probe must run and panic");
        stop_watch();
        assert_eq!(running_watcher_count(), 0);
        assert_eq!(
            has_panicked.load(Ordering::SeqCst),
            1,
            "the wrapper must not keep probing after the panic"
        );
        reset_state();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn stop_watch_is_bounded_against_a_hanging_probe_and_restart_never_overlaps() {
        let _serial = serialized_watch_tests();
        reset_state();
        let dir = std::env::temp_dir().join(format!("qol-watch-hang-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = hanging_modinfo_script(&dir);
        let concurrent = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        spawn_watcher(
            hanging_probe(script.clone(), started_tx, concurrent.clone(), peak.clone()),
            vec![qol_watch::WatchRoot::shallow(&dir)],
        );
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the hanging probe must start");

        let started = std::time::Instant::now();
        stop_watch();
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "stop must return within the deterministic bound while the probe subprocess would sleep 30 seconds"
        );
        assert_eq!(
            running_watcher_count(),
            0,
            "the contained generation must be joined and retired before stop returns"
        );

        let (restarted_tx, restarted_rx) = std::sync::mpsc::channel::<()>();
        spawn_watcher(
            hanging_probe(
                script.clone(),
                restarted_tx,
                concurrent.clone(),
                peak.clone(),
            ),
            vec![qol_watch::WatchRoot::shallow(&dir)],
        );
        restarted_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the restart must run a fresh bounded probe");
        stop_watch();
        assert_eq!(running_watcher_count(), 0);
        assert_eq!(
            peak.load(Ordering::SeqCst),
            1,
            "probes must never overlap across stop and restart"
        );
        reset_state();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_worker_panic_is_joined_and_the_generation_self_retires_without_a_second_stop() {
        let _serial = serialized_watch_tests();
        reset_state();
        let dir = std::env::temp_dir().join(format!("qol-watch-panic2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let concurrent = Arc::new(AtomicUsize::new(0));
        std::env::set_var("QOL_WATCH_WORKER_PANIC", "1");
        spawn_watcher(
            concurrency_watermark_probe(concurrent.clone(), Arc::new(AtomicUsize::new(0))),
            vec![qol_watch::WatchRoot::shallow(&dir)],
        );
        assert!(
            wait_until_idle(Duration::from_secs(5)),
            "the supervisor must join the panicked worker and self-retire the published generation without any stop"
        );
        std::env::remove_var("QOL_WATCH_WORKER_PANIC");
        assert_eq!(running_watcher_count(), 0);

        let started = std::time::Instant::now();
        stop_watch();
        assert!(
            started.elapsed() < STOP_BACKSTOP,
            "a stop after a panicked generation must not wait on a lost completion signal"
        );
        assert_eq!(running_watcher_count(), 0);
        reset_state();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_watcher_thread_spawn_failure_returns_and_leaves_the_lifecycle_idle() {
        let _serial = serialized_watch_tests();
        reset_state();
        let dir = std::env::temp_dir().join(format!("qol-watch-spawnfail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("QOL_WATCH_WORKER_SPAWN_FAILURE", "1");
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
        let spawner_dir = dir.clone();
        let spawner = std::thread::spawn(move || {
            spawn_watcher(
                Arc::new(move || {
                    let _ = started_tx.send(());
                    (Observation::NotLoaded, PolicyIntent::None)
                }),
                vec![qol_watch::WatchRoot::shallow(&spawner_dir)],
            );
            let _ = done_tx.send(());
        });
        done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect(
                "spawn_watcher must return when the watcher thread spawn fails instead of waiting on a readiness result that never arrives",
            );
        std::env::remove_var("QOL_WATCH_WORKER_SPAWN_FAILURE");
        assert!(
            started_rx.try_recv().is_err(),
            "the failed watcher spawn must never run a probe"
        );
        assert!(
            matches!(
                &*watch_state()
                    .0
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()),
                WatchState::Idle
            ),
            "the lifecycle must be Idle after the failed spawn, without any stop call"
        );
        spawner.join().unwrap();

        let (restarted_tx, restarted_rx) = std::sync::mpsc::channel::<()>();
        spawn_watcher(
            Arc::new(move || {
                let _ = restarted_tx.send(());
                (Observation::NotLoaded, PolicyIntent::None)
            }),
            vec![qol_watch::WatchRoot::shallow(&dir)],
        );
        restarted_rx.recv_timeout(Duration::from_secs(5)).expect(
            "a fresh spawn must work after the failed one, proving the supervisor finished",
        );
        stop_watch();
        assert_eq!(running_watcher_count(), 0);
        reset_state();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn stop_returns_bounded_while_a_gate_blocked_probe_outlives_the_backstop() {
        let _serial = serialized_watch_tests();
        reset_state();
        let dir = std::env::temp_dir().join(format!("qol-watch-gate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let gate = Arc::new(std::sync::Mutex::new(gate_rx));
        spawn_watcher(
            gated_probe(started_tx, gate.clone()),
            vec![qol_watch::WatchRoot::shallow(&dir)],
        );
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the gated probe must start and block");

        let started = std::time::Instant::now();
        stop_watch();
        assert!(
            started.elapsed() >= STOP_BACKSTOP
                && started.elapsed() < STOP_BACKSTOP + Duration::from_secs(2),
            "stop must return within the backstop bound while the probe is gate-blocked"
        );
        assert!(
            state_is_stopping(),
            "the unobserved-live generation must stay Stopping after the bounded stop"
        );

        let (restart_tx, restart_rx) = std::sync::mpsc::channel::<()>();
        let spawner_dir = dir.clone();
        let spawner = std::thread::spawn(move || {
            spawn_watcher(
                Arc::new(move || {
                    let _ = restart_tx.send(());
                    (Observation::NotLoaded, PolicyIntent::None)
                }),
                vec![qol_watch::WatchRoot::shallow(&spawner_dir)],
            );
        });
        assert!(
            restart_rx.recv_timeout(Duration::from_millis(300)).is_err(),
            "a replacement watcher must never overlap the unobserved-live generation"
        );

        let _ = gate_tx.send(());
        restart_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the replacement watcher must start automatically once the supervisor retires the old generation, without a second stop call");
        spawner.join().unwrap();
        assert_eq!(running_watcher_count(), 1);
        stop_watch();
        assert_eq!(running_watcher_count(), 0, "the lifecycle must end Idle");
        reset_state();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn concurrent_stops_are_bounded_and_finish_only_after_the_generation_exits() {
        let _serial = serialized_watch_tests();
        reset_state();
        let dir = std::env::temp_dir().join(format!("qol-watch-gate2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        let (gate_tx, gate_rx) = std::sync::mpsc::channel::<()>();
        let gate = Arc::new(std::sync::Mutex::new(gate_rx));
        spawn_watcher(
            gated_probe(started_tx, gate.clone()),
            vec![qol_watch::WatchRoot::shallow(&dir)],
        );
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the gated probe must start and block");

        let (finished_tx, finished_rx) = std::sync::mpsc::channel::<std::time::Duration>();
        let finished_tx_a = finished_tx.clone();
        let finished_tx_b = finished_tx.clone();
        let stopper_a = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            stop_watch();
            let elapsed = started.elapsed();
            let _ = finished_tx_a.send(elapsed);
        });
        let stopper_b = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            stop_watch();
            let elapsed = started.elapsed();
            let _ = finished_tx_b.send(elapsed);
        });
        for _ in 0..2 {
            let elapsed = finished_rx
                .recv_timeout(STOP_BACKSTOP + Duration::from_secs(2))
                .expect("every concurrent stop must return within the bounded backstop");
            assert!(
                elapsed < STOP_BACKSTOP + Duration::from_secs(2),
                "the concurrent stop exceeded the bound"
            );
        }
        assert!(
            state_is_stopping(),
            "concurrent stops must not join an unobserved-live generation"
        );
        stopper_a.join().unwrap();
        stopper_b.join().unwrap();

        let _ = gate_tx.send(());
        assert!(
            wait_until_idle(Duration::from_secs(5)),
            "the supervisor must retire the generation after the gate release"
        );
        assert_eq!(running_watcher_count(), 0, "the lifecycle must end Idle");
        reset_state();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn two_concurrent_stops_against_a_hanging_subprocess_both_return_bounded() {
        let _serial = serialized_watch_tests();
        reset_state();
        let dir = std::env::temp_dir().join(format!("qol-watch-twostop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = hanging_modinfo_script(&dir);
        let concurrent = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
        spawn_watcher(
            hanging_probe(script, started_tx, concurrent.clone(), peak.clone()),
            vec![qol_watch::WatchRoot::shallow(&dir)],
        );
        started_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the hanging probe must start");

        let (finished_tx, finished_rx) = std::sync::mpsc::channel::<std::time::Duration>();
        let finished_tx_a = finished_tx.clone();
        let finished_tx_b = finished_tx.clone();
        let stopper_a = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            stop_watch();
            let _ = finished_tx_a.send(started.elapsed());
        });
        let stopper_b = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            stop_watch();
            let _ = finished_tx_b.send(started.elapsed());
        });
        for _ in 0..2 {
            let elapsed = finished_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("every concurrent stop must return within the deterministic bound");
            assert!(elapsed < Duration::from_secs(5));
        }
        stopper_a.join().unwrap();
        stopper_b.join().unwrap();
        assert_eq!(
            running_watcher_count(),
            0,
            "the lifecycle must be Idle once every bounded stop returned"
        );
        assert_eq!(peak.load(Ordering::SeqCst), 1, "probes must never overlap");
        reset_state();
        std::fs::remove_dir_all(&dir).ok();
    }
}
