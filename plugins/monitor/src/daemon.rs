use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_runtime::protocol::DaemonRequest;
use qol_windowing::display::DisplayHandle;

use crate::config::{self, DeviceConfig};
use crate::monitor::{
    BrightnessState, DisplayControl, GammaStateControl, MonitorError, BRIGHTNESS_MAX,
    BRIGHTNESS_MIN, BRIGHTNESS_STEP,
};
use crate::platform::MonitorControl;
use crate::session::{LutProvider, RestoreMode, Session, SessionStore};

pub const HOLD_DEBOUNCE: Duration = Duration::from_millis(70);

const DAEMON_CONFIG: DaemonConfig = DaemonConfig {
    socket: SocketSource::EnvRequired,
    support_replace_existing: true,
};

static LIVE: OnceLock<Arc<Mutex<Runtime<dyn MonitorControl>>>> = OnceLock::new();

pub(crate) fn set_live_state(state: Arc<Mutex<Runtime<dyn MonitorControl>>>) {
    let _ = LIVE.set(state);
}

fn live_state() -> Option<&'static Arc<Mutex<Runtime<dyn MonitorControl>>>> {
    LIVE.get()
}

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
    ApplyPreferred,
    Reload,
    Kill,
}

pub fn parse_request(request: &DaemonRequest) -> ReadResult<Command> {
    match request.action.as_str() {
        "ping" => return ReadResult::Handled,
        "kill" => return ReadResult::Command(Command::Kill),
        "settings" => return ReadResult::Command(Command::Settings),
        "apply" => return ReadResult::Command(Command::ApplyPreferred),
        "reload" => return ReadResult::Command(Command::Reload),
        "displays" | "status" => return live_query(request.action.as_str()),
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

fn live_query(name: &str) -> ReadResult<Command> {
    let Some(live) = live_state() else {
        return ReadResult::Error("monitor daemon state is not ready".into());
    };
    let Ok(runtime) = live.lock() else {
        return ReadResult::Error("monitor daemon state is poisoned".into());
    };
    let control: &dyn MonitorControl = &**runtime.session().control();
    let payload = match name {
        "displays" => displays_payload(control, &runtime.config()),
        "status" => status_payload(control),
        _ => unreachable!("live_query only handles declared queries"),
    };
    ReadResult::HandledWithData(payload)
}

pub(crate) fn displays_payload(
    control: &dyn MonitorControl,
    config: &DeviceConfig,
) -> serde_json::Value {
    let rows: Vec<serde_json::Value> = control
        .enumerate()
        .unwrap_or_default()
        .iter()
        .map(|handle| {
            let id = handle.id().to_string();
            let policy = control.selection(handle.id()).label();
            let preferred = config.preferred_for(&id);
            let (brightness, source, detail) = match control.get_brightness(handle) {
                Ok(state) => (
                    serde_json::Value::from(state.value),
                    state.source.label(),
                    format!("{}% via {}", state.value, state.source.label()),
                ),
                Err(error) => (
                    serde_json::Value::Null,
                    "unavailable",
                    brightness_detail(&error),
                ),
            };
            serde_json::json!({
                "id": id,
                "connector": handle.connector(),
                "stable": !handle.identity_unstable(),
                "brightness": brightness,
                "source": source,
                "policy": policy,
                "preferred": preferred,
                "detail": detail,
            })
        })
        .collect();
    serde_json::json!(rows)
}

fn brightness_detail(error: &MonitorError) -> String {
    match error {
        MonitorError::Refused { reason, .. } => reason.clone(),
        _ => "unavailable".to_string(),
    }
}

pub(crate) fn status_payload(control: &dyn MonitorControl) -> serde_json::Value {
    let handles = control.enumerate().unwrap_or_default();
    if handles.is_empty() {
        return serde_json::json!({ "state": "no_displays", "count": 0 });
    }
    let readable: Vec<BrightnessState> = handles
        .iter()
        .filter_map(|handle| control.get_brightness(handle).ok())
        .collect();
    if readable.is_empty() {
        return serde_json::json!({ "state": "unavailable", "count": handles.len() });
    }
    let state = readable[0];
    serde_json::json!({
        "state": "ok",
        "count": readable.len(),
        "brightness": state.value,
        "source": state.source.label(),
    })
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

pub(crate) struct Runtime<C: DisplayControl + ?Sized> {
    session: Session<C>,
    stepper: HoldStepper,
    stop_requested: bool,
    notify: Notify,
    config: Arc<Mutex<DeviceConfig>>,
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
            config: Arc::new(Mutex::new(DeviceConfig::default())),
        }
    }

    pub fn session(&self) -> &Session<C> {
        &self.session
    }

    pub fn start(&mut self, config: &DeviceConfig) -> crate::session::RestoreReport {
        let recovery = self.session.restore_all(RestoreMode::Recovery);
        self.surface_gamma_warnings(&recovery);
        *self.config.lock().unwrap() = config.clone();
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

    fn apply_preferred(&self, preferred: &BTreeMap<String, config::BrightnessPreference>) -> usize {
        let Ok(handles) = self.session.control().enumerate() else {
            return 0;
        };
        let mut applied = 0;
        for handle in &handles {
            if handle.identity_unstable() {
                continue;
            }
            let Some(preference) = preferred.get(handle.id()) else {
                continue;
            };
            if self.session.mutate(handle, preference.brightness).is_ok() {
                applied += 1;
            }
        }
        applied
    }

    fn first_display(&self) -> Option<DisplayHandle> {
        self.session
            .control()
            .enumerate()
            .ok()
            .and_then(|handles| handles.into_iter().next())
    }
}

impl<C: DisplayControl + GammaStateControl + MonitorControl + ?Sized> Runtime<C> {
    pub fn config(&self) -> DeviceConfig {
        self.config.lock().unwrap().clone()
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
            Command::ApplyPreferred => {
                let config = self.config();
                let applied = self.apply_preferred(&config.preferred_brightness);
                self.notify_applied(applied);
                true
            }
            Command::Reload => {
                let next = config::load().unwrap_or_else(|error| {
                    eprintln!("[plugin-monitor] config reload failed: {error:#}");
                    DeviceConfig::default()
                });
                let applied = self.reload_config(&next);
                self.notify_applied(applied);
                true
            }
            Command::Kill => {
                let report = self.session.restore_all(RestoreMode::Exit);
                self.surface_gamma_warnings(&report);
                false
            }
        }
    }

    pub fn reload_config(&mut self, next: &DeviceConfig) -> usize {
        let previous = std::mem::replace(&mut *self.config.lock().unwrap(), next.clone());
        let stable_ids: HashSet<String> = self
            .session
            .control()
            .enumerate()
            .unwrap_or_default()
            .into_iter()
            .filter(|handle| !handle.identity_unstable())
            .map(|handle| handle.id().to_string())
            .collect();
        for display_id in stable_ids {
            self.session
                .control()
                .select(&display_id, next.policy_for(&display_id));
        }
        self.apply_preferred_deltas(&previous, next)
    }

    fn apply_preferred_deltas(&self, previous: &DeviceConfig, next: &DeviceConfig) -> usize {
        let Ok(handles) = self.session.control().enumerate() else {
            return 0;
        };
        let mut applied = 0;
        for handle in &handles {
            if handle.identity_unstable() {
                continue;
            }
            let old = previous.preferred_for(handle.id());
            let new = next.preferred_for(handle.id());
            if new == old {
                continue;
            }
            if let Some(value) = new {
                if self.session.mutate(handle, value).is_ok() {
                    applied += 1;
                }
            }
        }
        applied
    }

    fn notify_applied(&self, applied: usize) {
        if applied > 0 {
            (self.notify)(
                "Monitor",
                &format!(
                    "Applied preferred brightness to {applied} display{}",
                    if applied == 1 { "" } else { "s" }
                ),
            );
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

fn drain_trailing_heartbeats(rx: &Receiver<Command>) -> Option<Command> {
    loop {
        match rx.try_recv() {
            Ok(Command::Brightness {
                phase: Phase::Heartbeat,
                ..
            }) => {}
            Ok(other) => return Some(other),
            Err(_) => return None,
        }
    }
}

fn run_loop<C: DisplayControl + GammaStateControl + MonitorControl + ?Sized>(
    runtime: &Mutex<Runtime<C>>,
    rx: &Receiver<Command>,
) {
    while let Ok(Some(command)) = receive_commands(rx) {
        let mut runtime = runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if is_heartbeat(&command) {
            let carried = drain_trailing_heartbeats(rx);
            if !runtime.handle(command) {
                drain_all_queued(rx);
                break;
            }
            if let Some(carried) = carried {
                if !runtime.handle(carried) {
                    drain_all_queued(rx);
                    break;
                }
            }
        } else if !runtime.handle(command) {
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
    let (device, _origin) = config::load_with_origin(config_root.as_deref());
    let runtime = Arc::new(Mutex::new(build_runtime(config_root, &device)));
    let recovery = runtime.lock().unwrap().start(&device);
    set_live_state(Arc::clone(&runtime));
    let (tx, rx) = mpsc::channel();
    if !core_daemon::start_request_listener(&DAEMON_CONFIG, tx.clone(), parse_request) {
        return Err("failed to start plugin-monitor daemon listener".into());
    }
    trace_startup(&recovery);
    let _sigterm = install_sigterm_handler(tx);
    run_loop(&runtime, &rx);
    core_daemon::cleanup(&DAEMON_CONFIG);
    Ok(())
}

fn build_runtime(
    config_root: Option<PathBuf>,
    device: &DeviceConfig,
) -> Runtime<dyn MonitorControl> {
    let control = crate::platform::control();
    crate::platform::apply_configured_policies(&control, device);
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
    use crate::config::{BrightnessPreference, PolicySelection};
    use crate::monitor::{
        BrightnessPolicy, BrightnessSource, DisplayCapabilities, DisplayMode, GammaState, HdrState,
        RestoreOutcome,
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
    fn routes_ping_settings_kill_apply_reload_and_falls_back() {
        assert!(matches!(
            parse_request(&request("ping", serde_json::Value::Null)),
            ReadResult::Handled
        ));
        assert!(matches!(
            parse_request(&request("settings", serde_json::Value::Null)),
            ReadResult::Command(Command::Settings)
        ));
        assert!(matches!(
            parse_request(&request("apply", serde_json::Value::Null)),
            ReadResult::Command(Command::ApplyPreferred)
        ));
        assert!(matches!(
            parse_request(&request("reload", serde_json::Value::Null)),
            ReadResult::Command(Command::Reload)
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
        refuse_get: bool,
        selections: StdMutex<Vec<(String, BrightnessPolicy)>>,
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
                refuse_get: false,
                selections: StdMutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, u8)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl crate::platform::MonitorControl for FakeControl {
        fn select(&self, display_id: &str, policy: BrightnessPolicy) {
            self.selections
                .lock()
                .unwrap()
                .push((display_id.to_string(), policy));
        }

        fn selection(&self, display_id: &str) -> BrightnessPolicy {
            self.selections
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|(id, _)| id == display_id)
                .map(|(_, policy)| *policy)
                .unwrap_or_default()
        }

        fn gamma_backend(&self) -> Arc<dyn LutProvider> {
            Arc::new(NoLutProvider)
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
            Ok(RestoreOutcome::NothingToRestore)
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
            if self.refuse_get {
                return Err(MonitorError::refused(
                    "brightness",
                    "control is off for this display",
                ));
            }
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
            preferred_brightness: BTreeMap::from([(
                "id-1".to_string(),
                BrightnessPreference { brightness: 80 },
            )]),
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
            preferred_brightness: BTreeMap::from([(
                "id-1".to_string(),
                BrightnessPreference { brightness: 80 },
            )]),
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
            preferred_brightness: BTreeMap::from([(
                "id-1".to_string(),
                BrightnessPreference { brightness: 20 },
            )]),
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
    fn apply_preferred_command_applies_every_configured_display() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            60,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with(control.clone(), store);
        let config = DeviceConfig {
            preferred_brightness: BTreeMap::from([(
                "id-1".to_string(),
                BrightnessPreference { brightness: 85 },
            )]),
            ..DeviceConfig::default()
        };
        runtime.start(&config);
        control.calls.lock().unwrap().clear();
        assert!(runtime.handle(Command::ApplyPreferred));
        assert_eq!(control.calls(), vec![("id-1".to_string(), 85)]);
    }

    #[test]
    fn reload_config_applies_policy_and_preferred_deltas_only() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            60,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with(control.clone(), store);
        let initial = DeviceConfig {
            preferred_brightness: BTreeMap::from([(
                "id-1".to_string(),
                BrightnessPreference { brightness: 80 },
            )]),
            policy: BTreeMap::from([(
                "id-1".to_string(),
                PolicySelection {
                    policy: "ddc".into(),
                },
            )]),
        };
        runtime.start(&initial);
        control.calls.lock().unwrap().clear();
        let next = DeviceConfig {
            preferred_brightness: BTreeMap::from([(
                "id-1".to_string(),
                BrightnessPreference { brightness: 90 },
            )]),
            policy: BTreeMap::from([(
                "id-1".to_string(),
                PolicySelection {
                    policy: "gamma".into(),
                },
            )]),
        };
        assert_eq!(runtime.reload_config(&next), 1);
        assert_eq!(control.calls(), vec![("id-1".to_string(), 90)]);
        assert_eq!(control.selection("id-1"), BrightnessPolicy::Gamma);
        assert_eq!(runtime.config().preferred_for("id-1"), Some(90));
        control.calls.lock().unwrap().clear();
        assert_eq!(
            runtime.reload_config(&next),
            0,
            "an unchanged config applies nothing"
        );
        assert_eq!(control.calls(), Vec::<(String, u8)>::new());
    }

    #[test]
    fn reload_config_removing_entries_reverts_policy_and_never_touches_brightness() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            60,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with(control.clone(), store);
        let initial = DeviceConfig {
            preferred_brightness: BTreeMap::from([(
                "id-1".to_string(),
                BrightnessPreference { brightness: 80 },
            )]),
            policy: BTreeMap::from([(
                "id-1".to_string(),
                PolicySelection {
                    policy: "off".into(),
                },
            )]),
        };
        runtime.start(&initial);
        control.calls.lock().unwrap().clear();
        assert_eq!(runtime.reload_config(&DeviceConfig::default()), 0);
        assert_eq!(
            control.calls(),
            Vec::<(String, u8)>::new(),
            "removing a preference leaves the display where it is"
        );
        assert_eq!(
            control.selection("id-1"),
            BrightnessPolicy::Auto,
            "removing a policy entry reverts the display to auto"
        );
    }

    #[test]
    fn displays_payload_reports_live_state_and_config() {
        let control = FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            42,
            BrightnessSource::Ddc,
        );
        control.select("id-1", BrightnessPolicy::Gamma);
        let config = DeviceConfig {
            preferred_brightness: BTreeMap::from([(
                "id-1".to_string(),
                BrightnessPreference { brightness: 80 },
            )]),
            ..DeviceConfig::default()
        };
        let payload = displays_payload(&control, &config);
        assert_eq!(
            payload,
            serde_json::json!([
                {
                    "id": "id-1",
                    "connector": "card0-DP-1",
                    "stable": true,
                    "brightness": 42,
                    "source": "ddc",
                    "policy": "gamma",
                    "preferred": 80,
                    "detail": "42% via ddc",
                }
            ])
        );
    }

    #[test]
    fn displays_payload_reports_unreadable_brightness() {
        let mut control = FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            42,
            BrightnessSource::Ddc,
        );
        control.refuse_get = true;
        let payload = displays_payload(&control, &DeviceConfig::default());
        assert_eq!(payload[0]["brightness"], serde_json::Value::Null);
        assert_eq!(payload[0]["source"], "unavailable");
        assert_eq!(payload[0]["detail"], "control is off for this display");
        assert_eq!(payload[0]["preferred"], serde_json::Value::Null);
    }

    #[test]
    fn status_payload_maps_no_displays_ok_and_unavailable() {
        let empty = FakeControl::new(Vec::new(), 0, BrightnessSource::Ddc);
        assert_eq!(
            status_payload(&empty),
            serde_json::json!({ "state": "no_displays", "count": 0 })
        );
        let ok = FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            42,
            BrightnessSource::Ddc,
        );
        assert_eq!(
            status_payload(&ok),
            serde_json::json!({
                "state": "ok",
                "count": 1,
                "brightness": 42,
                "source": "ddc",
            })
        );
        let mut refused = FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            42,
            BrightnessSource::Ddc,
        );
        refused.refuse_get = true;
        assert_eq!(
            status_payload(&refused),
            serde_json::json!({ "state": "unavailable", "count": 1 })
        );
    }

    #[test]
    fn routes_queries_from_live_state_with_data() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            42,
            BrightnessSource::Ddc,
        ));
        let config = DeviceConfig {
            preferred_brightness: BTreeMap::from([(
                "id-1".to_string(),
                BrightnessPreference { brightness: 80 },
            )]),
            ..DeviceConfig::default()
        };
        let mut runtime: Runtime<dyn MonitorControl> = Runtime::new(
            control.clone(),
            store,
            Arc::new(NoLutProvider),
            |_title, _body| {},
        );
        runtime.start(&config);
        set_live_state(Arc::new(Mutex::new(runtime)));

        let ReadResult::HandledWithData(payload) =
            parse_request(&request("displays", serde_json::Value::Null))
        else {
            panic!("displays must answer with data");
        };
        assert_eq!(payload[0]["connector"], "card0-DP-1");
        assert_eq!(
            payload[0]["brightness"], 80,
            "start applies the preferred value"
        );
        assert_eq!(payload[0]["preferred"], 80);

        let ReadResult::HandledWithData(payload) =
            parse_request(&request("status", serde_json::Value::Null))
        else {
            panic!("status must answer with data");
        };
        assert_eq!(payload["state"], "ok");
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
        let runtime = Mutex::new(runtime_with(control.clone(), store));
        let (tx, rx) = mpsc::channel();
        let loop_thread = std::thread::spawn(move || run_loop(&runtime, &rx));
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
    fn queued_kill_behind_heartbeats_still_runs_the_exit_restore() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let runtime = Mutex::new(runtime_with(control.clone(), store.clone()));
        runtime
            .lock()
            .unwrap()
            .session()
            .mutate(&display, 60)
            .unwrap();
        control.calls.lock().unwrap().clear();
        let (tx, rx) = mpsc::channel();
        for command in [
            Command::Brightness {
                direction: -1,
                phase: Phase::Start,
            },
            Command::Brightness {
                direction: -1,
                phase: Phase::Heartbeat,
            },
            Command::Brightness {
                direction: -1,
                phase: Phase::Heartbeat,
            },
            Command::Kill,
        ] {
            tx.send(command).unwrap();
        }
        drop(tx);
        let loop_thread = std::thread::spawn(move || run_loop(&runtime, &rx));
        loop_thread.join().expect("the loop must exit");
        let snapshot = store.load_snapshot("id-1").unwrap().unwrap();
        assert!(snapshot.clean, "the queued kill must run the exit restore");
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 55), ("id-1".to_string(), 100)],
            "the kill behind the heartbeats steps once then restores"
        );
    }

    #[test]
    fn queued_stop_behind_heartbeats_halts_stepping() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let runtime = Mutex::new(runtime_with(control.clone(), store));
        let (tx, rx) = mpsc::channel();
        for command in [
            Command::Brightness {
                direction: -1,
                phase: Phase::Start,
            },
            Command::Brightness {
                direction: -1,
                phase: Phase::Heartbeat,
            },
            Command::Brightness {
                direction: -1,
                phase: Phase::Stop,
            },
            Command::Brightness {
                direction: -1,
                phase: Phase::Heartbeat,
            },
            Command::Brightness {
                direction: -1,
                phase: Phase::Heartbeat,
            },
        ] {
            tx.send(command).unwrap();
        }
        drop(tx);
        let loop_thread = std::thread::spawn(move || run_loop(&runtime, &rx));
        loop_thread.join().expect("the loop must exit");
        let calls = control.calls();
        assert!(
            calls == vec![("id-1".to_string(), 95)]
                || calls == vec![("id-1".to_string(), 95), ("id-1".to_string(), 90)],
            "steps must stop once the queued stop is consumed: {calls:?}"
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
        let runtime = Mutex::new(runtime_with(control.clone(), store.clone()));
        runtime
            .lock()
            .unwrap()
            .session()
            .mutate(&display, 60)
            .unwrap();
        control.calls.lock().unwrap().clear();
        let (tx, rx) = mpsc::channel();
        let _sigterm = install_sigterm_handler(tx);
        let loop_thread = std::thread::spawn(move || run_loop(&runtime, &rx));
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
