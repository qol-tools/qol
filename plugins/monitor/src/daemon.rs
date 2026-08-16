use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_runtime::protocol::DaemonRequest;
use qol_windowing::display::DisplayHandle;

use crate::config::{self, DeviceConfig};
use crate::monitor::{
    BrightnessPolicy, DisplayControl, GammaStateControl, BRIGHTNESS_MAX, BRIGHTNESS_MIN,
    BRIGHTNESS_STEP,
};
use crate::platform::MonitorControl;
use crate::session::{LutProvider, RestoreMode, Session, SessionStore};

pub const HOLD_DEBOUNCE: Duration = Duration::from_millis(70);

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Start,
    Heartbeat,
    Stop,
}

impl Phase {
    pub fn parse(input: &serde_json::Value) -> Result<Self, String> {
        match input.get("phase").and_then(serde_json::Value::as_str) {
            Some("start") => Ok(Self::Start),
            Some("heartbeat") => Ok(Self::Heartbeat),
            Some("stop") => Ok(Self::Stop),
            Some(phase) => Err(format!("Unknown continuous action phase: {phase}")),
            None => Err("Continuous action input requires a phase".into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Brightness { direction: i8, phase: Phase },
    Settings,
    Kill,
}

pub fn parse_request(request: &DaemonRequest) -> ReadResult<Command> {
    match request.action.as_str() {
        "ping" => return ReadResult::Handled,
        "kill" => return ReadResult::Command(Command::Kill),
        "settings" => return ReadResult::Command(Command::Settings),
        "brightness-up" => {
            return match Phase::parse(&request.input) {
                Ok(phase) => ReadResult::Command(Command::Brightness {
                    direction: 1,
                    phase,
                }),
                Err(error) => ReadResult::Error(error),
            };
        }
        "brightness-down" => {
            return match Phase::parse(&request.input) {
                Ok(phase) => ReadResult::Command(Command::Brightness {
                    direction: -1,
                    phase,
                }),
                Err(error) => ReadResult::Error(error),
            };
        }
        _ => {}
    }
    ReadResult::Fallback
}

pub fn step_value(current: u8, direction: i8) -> Option<u8> {
    let stepped = i16::from(current) + i16::from(direction) * i16::from(BRIGHTNESS_STEP);
    let next = stepped.clamp(i16::from(BRIGHTNESS_MIN), i16::from(BRIGHTNESS_MAX)) as u8;
    (next != current).then_some(next)
}

pub struct HoldStepper {
    debounce: Duration,
    last_step: Option<Instant>,
}

impl HoldStepper {
    pub fn new(debounce: Duration) -> Self {
        Self {
            debounce,
            last_step: None,
        }
    }

    pub fn gate(&mut self, phase: Phase, now: Instant) -> bool {
        match phase {
            Phase::Start => {
                self.last_step = Some(now);
                true
            }
            Phase::Heartbeat => match self.last_step {
                Some(last) if now.duration_since(last) >= self.debounce => {
                    self.last_step = Some(now);
                    true
                }
                Some(_) => false,
                None => {
                    self.last_step = Some(now);
                    true
                }
            },
            Phase::Stop => false,
        }
    }

    pub fn reset(&mut self) {
        self.last_step = None;
    }
}

pub struct Runtime<C: DisplayControl + ?Sized> {
    session: Session<C>,
    stepper: HoldStepper,
    stop_requested: bool,
    notify: Notify,
}

type Notify = Arc<dyn Fn(&str, &str) + Send + Sync>;

impl<C: DisplayControl + GammaStateControl + ?Sized> Runtime<C> {
    pub fn new(
        control: Arc<C>,
        store: SessionStore,
        lut: Arc<dyn LutProvider>,
        notify: impl Fn(&str, &str) + Send + Sync + 'static,
    ) -> Self {
        Self {
            session: Session::new(control, store, lut),
            stepper: HoldStepper::new(HOLD_DEBOUNCE),
            stop_requested: false,
            notify: Arc::new(notify),
        }
    }

    pub fn session(&self) -> &Session<C> {
        &self.session
    }

    pub fn start(&mut self, config: &DeviceConfig) -> crate::session::RestoreReport {
        let recovery = self.session.restore_all(RestoreMode::Recovery);
        self.surface_gamma_warnings(&recovery);
        self.apply_preferred(&config.preferred_brightness);
        recovery
    }

    fn surface_gamma_warnings(&self, report: &crate::session::RestoreReport) {
        if report.failed == 0 {
            return;
        }
        let Ok(handles) = self.session.control().enumerate() else {
            return;
        };
        for handle in handles {
            if self.session.control().warned(&handle) {
                (self.notify)(
                    "Monitor",
                    &format!(
                        "Brightness restore failed on {}: the gamma LUT is co-owned by another program",
                        handle.connector()
                    ),
                );
            }
        }
    }

    fn apply_preferred(&self, preferred: &BTreeMap<String, u8>) {
        let Ok(handles) = self.session.control().enumerate() else {
            return;
        };
        for handle in &handles {
            if handle.identity_unstable() {
                continue;
            }
            let Some(value) = preferred.get(handle.id()) else {
                continue;
            };
            if let Err(error) = self.session.mutate(handle, *value) {
                eprintln!(
                    "[plugin-monitor] preferred brightness {} for {} failed: {error}",
                    value,
                    handle.connector()
                );
            }
        }
    }

    pub fn handle(&mut self, command: Command) -> bool {
        match command {
            Command::Brightness { direction, phase } => {
                match phase {
                    Phase::Stop => {
                        self.stop_requested = true;
                        self.stepper.reset();
                    }
                    Phase::Start => {
                        self.stop_requested = false;
                        self.stepper.reset();
                    }
                    Phase::Heartbeat => {}
                }
                if self.stop_requested {
                    return true;
                }
                if self.stepper.gate(phase, Instant::now()) {
                    self.step(direction);
                }
                true
            }
            Command::Settings => {
                if let Err(error) =
                    qol_apps::desktop_integration::open_plugin_settings(crate::hotkeys::PLUGIN_ID)
                {
                    eprintln!("[plugin-monitor] failed to open settings page: {error}");
                }
                true
            }
            Command::Kill => {
                let report = self.session.restore_all(RestoreMode::Exit);
                self.surface_gamma_warnings(&report);
                false
            }
        }
    }

    fn step(&mut self, direction: i8) {
        let Some(handle) = self.first_display() else {
            return;
        };
        let Ok(state) = self.session.control().get_brightness(&handle) else {
            return;
        };
        let Some(next) = step_value(state.value, direction) else {
            return;
        };
        if self.session.mutate(&handle, next).is_err() {
            return;
        }
        let source = self
            .session
            .control()
            .get_brightness(&handle)
            .map(|state| state.source)
            .unwrap_or(state.source);
        (self.notify)(
            "Monitor",
            &format!("Brightness {}% ({})", next, source.label()),
        );
    }

    fn first_display(&self) -> Option<DisplayHandle> {
        self.session
            .control()
            .enumerate()
            .ok()
            .and_then(|handles| handles.into_iter().next())
    }
}

fn receive_commands(rx: &Receiver<Command>) -> Result<Option<Command>, ()> {
    rx.recv().map(Some).map_err(|_| ())
}

fn is_heartbeat(command: &Command) -> bool {
    matches!(
        command,
        Command::Brightness {
            phase: Phase::Heartbeat,
            ..
        }
    )
}

fn drain_trailing_heartbeats(rx: &Receiver<Command>) {
    while let Ok(Command::Brightness {
        phase: Phase::Heartbeat,
        ..
    }) = rx.try_recv()
    {}
}

fn run_loop<C: DisplayControl + GammaStateControl + ?Sized>(
    runtime: &mut Runtime<C>,
    rx: &Receiver<Command>,
) {
    while let Ok(Some(command)) = receive_commands(rx) {
        if is_heartbeat(&command) {
            drain_trailing_heartbeats(rx);
        }
        if !runtime.handle(command) {
            drain_all_queued(rx);
            break;
        }
    }
}

fn drain_all_queued(rx: &Receiver<Command>) {
    while rx.try_recv().is_ok() {}
}

fn install_sigterm_handler(tx: Sender<Command>) -> signal_hook::iterator::Handle {
    let mut signals = signal_hook::iterator::Signals::new([signal_hook::consts::SIGTERM])
        .expect("failed to register the SIGTERM handler");
    let handle = signals.handle();
    std::thread::Builder::new()
        .name("monitor-sigterm".into())
        .spawn(move || {
            if let Some(signal) = signals.forever().next() {
                if signal == signal_hook::consts::SIGTERM {
                    let _ = tx.send(Command::Kill);
                }
            }
        })
        .expect("failed to spawn the SIGTERM forwarder");
    handle
}

pub fn run() -> Result<(), String> {
    if std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET).is_none() {
        return Err(format!(
            "{} is not set; run the daemon through qol-tray",
            qol_conventions::ENV_DAEMON_SOCKET
        ));
    }
    let config_root = config::config_root();
    let device = config::load(config_root.as_deref().unwrap_or(std::path::Path::new("")))
        .unwrap_or_else(|error| {
            eprintln!("[plugin-monitor] device config unreadable: {error:#}");
            DeviceConfig::default()
        });
    let mut runtime = build_runtime(config_root, &device);
    let (tx, rx) = mpsc::channel();
    if !core_daemon::start_request_listener(&DAEMON_CONFIG, tx.clone(), parse_request) {
        return Err("failed to start plugin-monitor daemon listener".into());
    }
    let recovery = runtime.start(&device);
    trace_startup(&recovery);
    let _sigterm = install_sigterm_handler(tx);
    run_loop(&mut runtime, &rx);
    core_daemon::cleanup(&DAEMON_CONFIG);
    Ok(())
}

fn build_runtime(
    config_root: Option<PathBuf>,
    device: &DeviceConfig,
) -> Runtime<dyn MonitorControl> {
    let control = crate::platform::control();
    let stable_ids: HashSet<String> = control
        .enumerate()
        .unwrap_or_default()
        .into_iter()
        .filter(|handle| !handle.identity_unstable())
        .map(|handle| handle.id().to_string())
        .collect();
    for (display_id, label) in &device.policy {
        if stable_ids.contains(display_id) {
            if let Some(policy) = BrightnessPolicy::parse(label) {
                control.select(display_id, policy);
            }
        }
    }
    let store = session_store_for(config_root.as_deref());
    let lut: Arc<dyn LutProvider> = control.gamma_backend();
    Runtime::new(control, store, lut, notify)
}

fn session_store_for(config_root: Option<&std::path::Path>) -> SessionStore {
    let dir = config_root
        .and_then(|root| config::session_dir(root).ok())
        .unwrap_or_else(fallback_session_dir);
    SessionStore::new(dir)
}

fn fallback_session_dir() -> PathBuf {
    let fallback = std::env::temp_dir().join("qol-monitor-session");
    if let Err(error) = qol_fs::create_private_dir(&fallback) {
        eprintln!("[plugin-monitor] cannot secure the fallback session dir: {error}");
    }
    fallback
}

fn notify(title: &str, message: &str) {
    qol_plugin_daemon::notification::send_notification(title, message);
}

fn trace_startup(recovery: &crate::session::RestoreReport) {
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "MONITOR_SESSION",
        "event=start restored={} preserved={} gone={} failed={}",
        recovery.restored,
        recovery.foreign_lut_preserved,
        recovery.skipped_display_gone,
        recovery.failed
    );
    #[cfg(not(debug_assertions))]
    let _ = recovery;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::monitor::{
        BrightnessSource, BrightnessState, DisplayCapabilities, DisplayMode, GammaState,
        GammaStateControl, HdrState, MonitorError,
    };
    use crate::session::NoLutProvider;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;

    fn handle(id: &str, connector: &str) -> DisplayHandle {
        DisplayHandle::new(id.into(), connector.into(), None, false)
    }

    fn request(action: &str, input: serde_json::Value) -> DaemonRequest {
        DaemonRequest {
            action: action.into(),
            input,
        }
    }

    #[test]
    fn parses_continuous_action_phases() {
        assert!(matches!(
            parse_request(&request(
                "brightness-up",
                serde_json::json!({ "phase": "start" })
            )),
            ReadResult::Command(Command::Brightness {
                direction: 1,
                phase: Phase::Start
            })
        ));
        assert!(matches!(
            parse_request(&request(
                "brightness-down",
                serde_json::json!({ "phase": "heartbeat" })
            )),
            ReadResult::Command(Command::Brightness {
                direction: -1,
                phase: Phase::Heartbeat
            })
        ));
        assert!(matches!(
            parse_request(&request(
                "brightness-up",
                serde_json::json!({ "phase": "stop" })
            )),
            ReadResult::Command(Command::Brightness {
                direction: 1,
                phase: Phase::Stop
            })
        ));
    }

    #[test]
    fn rejects_continuous_actions_without_a_phase() {
        assert!(matches!(
            parse_request(&request("brightness-up", serde_json::Value::Null)),
            ReadResult::Error(_)
        ));
    }

    #[test]
    fn routes_ping_settings_kill_and_falls_back() {
        assert!(matches!(
            parse_request(&request("ping", serde_json::Value::Null)),
            ReadResult::Handled
        ));
        assert!(matches!(
            parse_request(&request("settings", serde_json::Value::Null)),
            ReadResult::Command(Command::Settings)
        ));
        assert!(matches!(
            parse_request(&request("kill", serde_json::Value::Null)),
            ReadResult::Command(Command::Kill)
        ));
        assert!(matches!(
            parse_request(&request("nope", serde_json::Value::Null)),
            ReadResult::Fallback
        ));
    }

    #[test]
    fn steps_by_five_and_clamps_at_the_bounds() {
        assert_eq!(step_value(42, 1), Some(47));
        assert_eq!(step_value(42, -1), Some(37));
        assert_eq!(step_value(99, 1), Some(100));
        assert_eq!(step_value(100, 1), None, "clamped at max is a no-op");
        assert_eq!(step_value(2, -1), Some(0));
        assert_eq!(step_value(0, -1), None, "clamped at min is a no-op");
        assert_eq!(step_value(95, 1), Some(100));
    }

    #[test]
    fn hold_stepper_gates_repeats_at_the_debounce_interval() {
        let debounce = Duration::from_millis(70);
        let mut stepper = HoldStepper::new(debounce);
        let now = Instant::now();
        assert!(stepper.gate(Phase::Start, now), "start steps immediately");
        assert!(
            !stepper.gate(Phase::Heartbeat, now + Duration::from_millis(69)),
            "a repeat inside the debounce window is dropped"
        );
        assert!(
            stepper.gate(Phase::Heartbeat, now + Duration::from_millis(70)),
            "a repeat at the debounce boundary steps"
        );
        assert!(
            !stepper.gate(Phase::Stop, now + Duration::from_millis(1000)),
            "stop never steps"
        );
        stepper.reset();
        assert!(
            stepper.gate(Phase::Heartbeat, Instant::now() + Duration::from_millis(1)),
            "a heartbeat without a start still steps"
        );
    }

    struct FakeControl {
        displays: Vec<DisplayHandle>,
        current: StdMutex<u8>,
        source: BrightnessSource,
        calls: StdMutex<Vec<(String, u8)>>,
        steps: AtomicUsize,
        warned: StdMutex<bool>,
    }

    impl FakeControl {
        fn new(displays: Vec<DisplayHandle>, current: u8, source: BrightnessSource) -> Self {
            Self {
                displays,
                current: StdMutex::new(current),
                source,
                calls: StdMutex::new(Vec::new()),
                steps: AtomicUsize::new(0),
                warned: StdMutex::new(false),
            }
        }

        fn calls(&self) -> Vec<(String, u8)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl GammaStateControl for FakeControl {
        fn mismatch_count(&self, _handle: &DisplayHandle) -> usize {
            0
        }

        fn warned(&self, _handle: &DisplayHandle) -> bool {
            *self.warned.lock().unwrap()
        }

        fn restore(
            &self,
            _handle: &DisplayHandle,
        ) -> Result<crate::monitor::RestoreOutcome, MonitorError> {
            Ok(crate::monitor::RestoreOutcome::NothingToRestore)
        }
    }

    impl DisplayControl for FakeControl {
        fn enumerate(&self) -> Result<Vec<DisplayHandle>, MonitorError> {
            Ok(self.displays.clone())
        }

        fn probe(&self, _handle: &DisplayHandle) -> Result<DisplayCapabilities, MonitorError> {
            Ok(DisplayCapabilities::none())
        }

        fn get_brightness(&self, _handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
            Ok(BrightnessState {
                value: *self.current.lock().unwrap(),
                source: self.source,
            })
        }

        fn set_brightness(&self, _handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
            self.steps.fetch_add(1, Ordering::SeqCst);
            *self.current.lock().unwrap() = value;
            self.calls
                .lock()
                .unwrap()
                .push((_handle.id().to_string(), value));
            Ok(())
        }

        fn get_gamma(&self, _handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
            Err(MonitorError::unsupported("gamma", "test"))
        }

        fn set_gamma(&self, _handle: &DisplayHandle, _value: u8) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("gamma", "test"))
        }

        fn list_modes(&self, _handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
            Err(MonitorError::unsupported("modes", "test"))
        }

        fn set_mode(
            &self,
            _handle: &DisplayHandle,
            _mode: &DisplayMode,
        ) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("modes", "test"))
        }

        fn get_hdr(&self, _handle: &DisplayHandle) -> Result<HdrState, MonitorError> {
            Err(MonitorError::unsupported("hdr", "test"))
        }

        fn set_hdr(&self, _handle: &DisplayHandle, _enabled: bool) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("hdr", "test"))
        }
    }

    fn runtime_with(control: Arc<FakeControl>, store: SessionStore) -> Runtime<FakeControl> {
        Runtime::new(control, store, Arc::new(NoLutProvider), |_title, _body| {})
    }

    fn runtime_store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().join("session"));
        (dir, store)
    }

    fn stale_snapshot(
        display_id: &str,
        connector: &str,
        value: u8,
        last_value: u8,
    ) -> crate::session::Snapshot {
        crate::session::Snapshot {
            schema_version: crate::session::SNAPSHOT_SCHEMA_VERSION,
            session_id: "00000000-0000-4000-8000-000000000000".into(),
            display_id: display_id.into(),
            connector: connector.into(),
            value,
            source: "ddc".into(),
            last_value,
            mutations: 3,
            clean: false,
            lut: None,
            checksum: String::new(),
        }
    }

    #[test]
    fn hotkey_steps_toast_value_and_source_and_clamp_at_bounds() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            95,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with(control.clone(), store);
        runtime.handle(Command::Brightness {
            direction: 1,
            phase: Phase::Start,
        });
        assert_eq!(control.calls(), vec![("id-1".to_string(), 100)]);
        runtime.handle(Command::Brightness {
            direction: 1,
            phase: Phase::Start,
        });
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 100)],
            "clamped at max does not step"
        );
    }

    #[test]
    fn start_restores_stale_snapshots_before_applying_preferred() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            60,
            BrightnessSource::Ddc,
        ));
        store
            .write_snapshot(&stale_snapshot("id-1", "card0-DP-1", 100, 60))
            .unwrap();
        let mut runtime = runtime_with(control.clone(), store);
        let preferred = DeviceConfig {
            preferred_brightness: BTreeMap::from([("id-1".to_string(), 80)]),
            ..DeviceConfig::default()
        };
        runtime.start(&preferred);
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 100), ("id-1".to_string(), 80),],
            "crash restore runs first, then preferred as a fresh snapshot"
        );
        let snapshot = runtime
            .session()
            .store()
            .load_snapshot("id-1")
            .unwrap()
            .unwrap();
        assert_eq!(
            snapshot.value, 100,
            "preferred snapshots the restored value"
        );
        assert_eq!(snapshot.last_value, 80);
    }

    #[test]
    fn exit_restores_after_preferred_and_is_idempotent() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with(control.clone(), store);
        let preferred = DeviceConfig {
            preferred_brightness: BTreeMap::from([("id-1".to_string(), 80)]),
            ..DeviceConfig::default()
        };
        runtime.start(&preferred);
        control.calls.lock().unwrap().clear();
        assert!(!runtime.handle(Command::Kill), "kill stops the loop");
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 100)],
            "exit restore returns to the pre-daemon state"
        );
        let snapshot = runtime
            .session()
            .store()
            .load_snapshot("id-1")
            .unwrap()
            .unwrap();
        assert!(snapshot.clean, "exit restore leaves the clean-exit marker");
        control.calls.lock().unwrap().clear();
        runtime.session().restore_all(RestoreMode::Exit);
        assert_eq!(
            control.calls(),
            Vec::<(String, u8)>::new(),
            "a second exit restore is a no-op"
        );
    }

    #[test]
    fn config_never_overrides_restore() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            60,
            BrightnessSource::Ddc,
        ));
        store
            .write_snapshot(&stale_snapshot("id-1", "card0-DP-1", 50, 60))
            .unwrap();
        let mut runtime = runtime_with(control.clone(), store);
        let preferred = DeviceConfig {
            preferred_brightness: BTreeMap::from([("id-1".to_string(), 20)]),
            ..DeviceConfig::default()
        };
        runtime.start(&preferred);
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 50), ("id-1".to_string(), 20),],
            "the crash-restored value is the snapshot base; preferred is a mutation on top"
        );
    }

    #[test]
    fn preferred_is_applied_only_to_configured_displays() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with(control.clone(), store);
        runtime.start(&DeviceConfig::default());
        assert_eq!(control.calls(), Vec::<(String, u8)>::new());
        assert_eq!(control.steps.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn queued_heartbeats_never_step_after_stop() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with(control.clone(), store);
        let (tx, rx) = mpsc::channel();
        let loop_thread = std::thread::spawn(move || run_loop(&mut runtime, &rx));
        tx.send(Command::Brightness {
            direction: -1,
            phase: Phase::Start,
        })
        .unwrap();
        while control.calls().is_empty() {
            std::thread::yield_now();
        }
        std::thread::sleep(HOLD_DEBOUNCE + Duration::from_millis(10));
        for _ in 0..3 {
            tx.send(Command::Brightness {
                direction: -1,
                phase: Phase::Heartbeat,
            })
            .unwrap();
        }
        tx.send(Command::Brightness {
            direction: -1,
            phase: Phase::Stop,
        })
        .unwrap();
        for _ in 0..3 {
            tx.send(Command::Brightness {
                direction: -1,
                phase: Phase::Heartbeat,
            })
            .unwrap();
        }
        drop(tx);
        loop_thread.join().expect("the loop must exit");
        assert_eq!(
            control.calls(),
            vec![
                ("id-1".to_string(), 95),
                ("id-1".to_string(), 90),
            ],
            "start steps once, one heartbeat steps, trailing heartbeats coalesce and stop halts stepping"
        );
    }

    #[test]
    fn sigterm_runs_the_exit_restore_and_marks_clean() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with(control.clone(), store.clone());
        runtime.session().mutate(&display, 60).unwrap();
        control.calls.lock().unwrap().clear();
        let (tx, rx) = mpsc::channel();
        let _sigterm = install_sigterm_handler(tx);
        let loop_thread = std::thread::spawn(move || run_loop(&mut runtime, &rx));
        let status = std::process::Command::new("kill")
            .args(["-TERM", &std::process::id().to_string()])
            .status()
            .expect("kill must run");
        assert!(status.success());
        loop_thread
            .join()
            .expect("the loop must exit after SIGTERM");
        let snap = store.load_snapshot("id-1").unwrap().unwrap();
        assert!(snap.clean, "SIGTERM must run the exit restore");
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 100)],
            "the SIGTERM restore returns to the pre-daemon state"
        );
    }

    #[test]
    fn kill_surfaces_the_gamma_mismatch_warning_after_a_failed_restore() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let mut control = FakeControl::new(vec![display.clone()], 60, BrightnessSource::Gamma);
        control.warned = StdMutex::new(true);
        let control = Arc::new(control);
        let notified = Arc::new(StdMutex::new(Vec::<String>::new()));
        let notify = {
            let notified = notified.clone();
            move |_title: &str, body: &str| notified.lock().unwrap().push(body.to_string())
        };
        let mut runtime = Runtime::new(control, store.clone(), Arc::new(NoLutProvider), notify);
        store
            .write_snapshot(&crate::session::Snapshot {
                source: "gamma".into(),
                lut: Some(crate::monitor::GammaTable {
                    red: vec![1000, 1000],
                    green: vec![1000, 1000],
                    blue: vec![1000, 1000],
                }),
                last_value: 60,
                ..stale_snapshot("id-1", "card0-DP-1", 100, 60)
            })
            .unwrap();
        runtime.handle(Command::Kill);
        let bodies = notified.lock().unwrap();
        assert!(
            bodies
                .iter()
                .any(|body| body.contains("gamma LUT is co-owned")),
            "the warn-at-3 mismatch must surface through the production restore path: {bodies:?}"
        );
    }

    #[test]
    fn successful_exit_restore_sends_no_mismatch_warning() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let notified = Arc::new(StdMutex::new(0usize));
        let notify = {
            let notified = notified.clone();
            move |_title: &str, _body: &str| *notified.lock().unwrap() += 1
        };
        let mut runtime = Runtime::new(control, store, Arc::new(NoLutProvider), notify);
        runtime.session().mutate(&display, 60).unwrap();
        runtime.handle(Command::Kill);
        assert_eq!(
            *notified.lock().unwrap(),
            0,
            "a clean exit restore must not warn"
        );
    }

    #[test]
    fn fallback_session_dir_is_created_private() {
        use std::os::unix::fs::PermissionsExt;

        let dir = fallback_session_dir();
        assert!(dir.is_dir(), "{} must exist", dir.display());
        let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "the /tmp fallback session dir must be private");
    }
}
