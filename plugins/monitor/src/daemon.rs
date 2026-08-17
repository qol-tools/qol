use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_runtime::protocol::DaemonRequest;

use crate::config::{self, DeviceConfig};
use crate::monitor::{
    BrightnessPolicy, BrightnessSource, BrightnessState, DisplayControl, GammaStateControl,
    MonitorError, BRIGHTNESS_MAX, BRIGHTNESS_MIN, BRIGHTNESS_STEP,
};
use crate::platform::MonitorControl;
use crate::session::{LutProvider, RestoreMode, Session, SessionStore};
use qol_windowing::display::DisplayHandle;

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
    SetBrightness { display: String, value: u8 },
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
        "set_brightness" => {
            return match parse_brightness_input(&request.input) {
                Ok((display, value)) => {
                    ReadResult::Command(Command::SetBrightness { display, value })
                }
                Err(error) => ReadResult::Error(error),
            };
        }
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

fn parse_brightness_input(input: &serde_json::Value) -> Result<(String, u8), String> {
    let id = input
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| "set_brightness input requires a display id".to_string())?
        .to_string();
    let value = input
        .get("value")
        .and_then(serde_json::Value::as_u64)
        .and_then(|raw| u8::try_from(raw).ok())
        .filter(|value| *value <= BRIGHTNESS_MAX)
        .ok_or_else(|| {
            format!(
                "set_brightness input requires a value between {} and {BRIGHTNESS_MAX}",
                BRIGHTNESS_MIN
            )
        })?;
    Ok((id, value))
}

fn live_query(name: &str) -> ReadResult<Command> {
    let Some(live) = live_state() else {
        return ReadResult::Error("monitor daemon state is not ready".into());
    };
    let Ok(mut runtime) = live.lock() else {
        return ReadResult::Error("monitor daemon state is poisoned".into());
    };
    let control = Arc::clone(runtime.session().control());
    let preferred = runtime.preferred.clone();
    let payload = match name {
        "displays" => displays_payload(&*control, &preferred, &mut runtime.brightness_cache),
        "status" => status_payload(&*control, &mut runtime.brightness_cache),
        _ => unreachable!("live_query only handles declared queries"),
    };
    ReadResult::HandledWithData(payload)
}

fn policy_config(config: &DeviceConfig) -> DeviceConfig {
    let mut policy_only = config.clone();
    policy_only.preferred_brightness.clear();
    policy_only
}

pub(crate) fn displays_payload(
    control: &dyn MonitorControl,
    preferred: &BTreeMap<String, u8>,
    cache: &mut BTreeMap<String, BrightnessState>,
) -> serde_json::Value {
    let handles = control.enumerate().unwrap_or_default();
    let rows: Vec<serde_json::Value> = handles
        .iter()
        .map(|handle| {
            let id = handle.id().to_string();
            let policy = control.selection(handle.id()).label();
            let preferred = preferred.get(&id).copied();
            let (brightness, source, detail) = match cache.get(handle.id()).copied() {
                Some(state) => (
                    serde_json::Value::from(state.value),
                    state.source.label(),
                    format!("{}% via {}", state.value, state.source.label()),
                ),
                None => match control.get_brightness(handle) {
                    Ok(state) => {
                        cache.insert(handle.id().to_string(), state);
                        (
                            serde_json::Value::from(state.value),
                            state.source.label(),
                            format!("{}% via {}", state.value, state.source.label()),
                        )
                    }
                    Err(error) => (
                        serde_json::Value::Null,
                        "unavailable",
                        brightness_detail(&error),
                    ),
                },
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
    cache.retain(|id, _| handles.iter().any(|handle| handle.id() == id.as_str()));
    serde_json::json!(rows)
}

fn brightness_detail(error: &MonitorError) -> String {
    match error {
        MonitorError::Refused { reason, .. } => reason.clone(),
        _ => "unavailable".to_string(),
    }
}

pub(crate) fn status_payload(
    control: &dyn MonitorControl,
    cache: &mut BTreeMap<String, BrightnessState>,
) -> serde_json::Value {
    let handles = control.enumerate().unwrap_or_default();
    if handles.is_empty() {
        return serde_json::json!({ "state": "no_displays", "count": 0 });
    }
    let readable: Vec<BrightnessState> = handles
        .iter()
        .filter_map(|handle| match cache.get(handle.id()).copied() {
            Some(state) => Some(state),
            None => control.get_brightness(handle).ok().inspect(|&state| {
                cache.insert(handle.id().to_string(), state);
            }),
        })
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
    preferred: BTreeMap<String, u8>,
    config_root: Option<PathBuf>,
    preferred_save: PreferredSave,
    brightness_cache: BTreeMap<String, BrightnessState>,
}

type Notify = Arc<dyn Fn(&str, &str) + Send + Sync>;
type PreferredSave = Arc<dyn Fn(&BTreeMap<String, u8>) -> anyhow::Result<()> + Send + Sync>;

impl<C: DisplayControl + GammaStateControl + ?Sized> Runtime<C> {
    pub fn new(
        control: Arc<C>,
        store: SessionStore,
        lut: Arc<dyn LutProvider>,
        notify: impl Fn(&str, &str) + Send + Sync + 'static,
        config_root: Option<PathBuf>,
        preferred_save: impl Fn(&BTreeMap<String, u8>) -> anyhow::Result<()> + Send + Sync + 'static,
    ) -> Self {
        Self {
            session: Session::new(control, store, lut),
            stepper: HoldStepper::new(HOLD_DEBOUNCE),
            stop_requested: false,
            notify: Arc::new(notify),
            config: Arc::new(Mutex::new(DeviceConfig::default())),
            preferred: BTreeMap::new(),
            config_root,
            preferred_save: Arc::new(preferred_save),
            brightness_cache: BTreeMap::new(),
        }
    }

    pub fn session(&self) -> &Session<C> {
        &self.session
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
}

impl<C: DisplayControl + GammaStateControl + MonitorControl + ?Sized> Runtime<C> {
    pub fn start(&mut self, config: &DeviceConfig) -> crate::session::RestoreReport {
        let recovery = self.session.restore_all(RestoreMode::Recovery);
        self.surface_gamma_warnings(&recovery);
        *self.config.lock().unwrap() = policy_config(config);
        self.preferred = config::load_preferred(self.config_root.as_deref());
        recovery
    }

    pub fn config(&self) -> DeviceConfig {
        self.config.lock().unwrap().clone()
    }

    fn apply_preferred_map(&mut self) -> usize {
        let Ok(handles) = self.session.control().enumerate() else {
            return 0;
        };
        let mut applied = 0;
        for handle in &handles {
            if handle.identity_unstable() {
                continue;
            }
            let Some(value) = self.preferred.get(handle.id()) else {
                continue;
            };
            if self.session.mutate(handle, *value).is_ok() {
                self.remember_brightness(handle, *value);
                applied += 1;
            }
        }
        applied
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
            Command::SetBrightness { display, value } => {
                let applied = self.set_brightness(&display, value);
                if applied.is_empty() {
                    (self.notify)(
                        "Monitor",
                        "Brightness could not be set on the selected display",
                    );
                    return true;
                }
                for id in &applied {
                    self.preferred.insert(id.clone(), value);
                }
                if let Err(error) = (self.preferred_save)(&self.preferred) {
                    eprintln!("[plugin-monitor] failed to persist preferred brightness: {error:#}");
                }
                if self.config().notify_on_change {
                    (self.notify)("Monitor", &format!("Brightness {value}%"));
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
                self.preferred = config::load_preferred(self.config_root.as_deref());
                let applied = self.apply_preferred_map();
                self.notify_applied(applied);
                true
            }
            Command::Reload => {
                let next = config::load().unwrap_or_else(|error| {
                    eprintln!("[plugin-monitor] config reload failed: {error:#}");
                    DeviceConfig::default()
                });
                self.preferred = config::load_preferred(self.config_root.as_deref());
                self.reload_config(&next);
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
        let previous = std::mem::replace(&mut *self.config.lock().unwrap(), policy_config(next));
        let stable_ids: HashSet<String> = self
            .session
            .control()
            .enumerate()
            .unwrap_or_default()
            .into_iter()
            .filter(|handle| !handle.identity_unstable())
            .map(|handle| handle.id().to_string())
            .collect();
        let mut changed = 0;
        for display_id in stable_ids {
            let policy = next.policy_for(&display_id);
            if previous.policy_for(&display_id) == policy {
                continue;
            }
            self.session.control().select(&display_id, policy);
            changed += 1;
        }
        changed
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

    fn set_brightness(&mut self, display: &str, value: u8) -> Vec<String> {
        let handles = self.session.control().enumerate().unwrap_or_default();
        let targets: Vec<&DisplayHandle> = if display == "all" {
            handles.iter().collect()
        } else {
            handles
                .iter()
                .filter(|handle| handle.id() == display)
                .collect()
        };
        let mut applied = Vec::new();
        for handle in targets {
            if self.session.mutate(handle, value).is_ok() {
                self.remember_brightness(handle, value);
                applied.push(handle.id().to_string());
            }
        }
        applied
    }

    fn step(&mut self, direction: i8) {
        let handles = self.session.control().enumerate().unwrap_or_default();
        let mut stepped: Vec<(u8, &'static str)> = Vec::new();
        for handle in &handles {
            let Some(current) = self.cached_brightness(handle) else {
                continue;
            };
            let Some(next) = step_value(current.value, direction) else {
                continue;
            };
            if self.session.mutate(handle, next).is_err() {
                continue;
            }
            self.brightness_cache.insert(
                handle.id().to_string(),
                BrightnessState {
                    value: next,
                    source: current.source,
                },
            );
            stepped.push((next, current.source.label()));
        }
        let message = match stepped.as_slice() {
            [] => return,
            [(value, source)] => format!("Brightness {value}% ({source})"),
            many => format!(
                "Brightness {} on {} displays",
                if direction > 0 { "up" } else { "down" },
                many.len()
            ),
        };
        if self.config().notify_on_change {
            (self.notify)("Monitor", &message);
        }
    }

    fn cached_brightness(&mut self, handle: &DisplayHandle) -> Option<BrightnessState> {
        if let Some(state) = self.brightness_cache.get(handle.id()) {
            return Some(*state);
        }
        let state = self.session.control().get_brightness(handle).ok()?;
        self.brightness_cache.insert(handle.id().to_string(), state);
        Some(state)
    }

    fn remember_brightness(&mut self, handle: &DisplayHandle, value: u8) {
        let source = self
            .brightness_cache
            .get(handle.id())
            .map(|state| state.source)
            .unwrap_or_else(|| self.write_source(handle));
        self.brightness_cache
            .insert(handle.id().to_string(), BrightnessState { value, source });
    }

    fn write_source(&self, handle: &DisplayHandle) -> BrightnessSource {
        match self.session.control().selection(handle.id()) {
            BrightnessPolicy::Gamma => BrightnessSource::Gamma,
            _ => BrightnessSource::Ddc,
        }
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
    let preferred_save = {
        let root = config_root.clone();
        move |preferred: &BTreeMap<String, u8>| config::save_preferred(root.as_deref(), preferred)
    };
    Runtime::new(control, store, lut, notify, config_root, preferred_save)
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
    use crate::config::{self, BrightnessPreference, PolicySelection};
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
    fn parses_set_brightness_with_an_id_and_value() {
        assert!(matches!(
            parse_request(&request(
                "set_brightness",
                serde_json::json!({ "id": "id-1", "value": 45 })
            )),
            ReadResult::Command(Command::SetBrightness { display, value })
                if display == "id-1" && value == 45
        ));
        assert!(matches!(
            parse_request(&request(
                "set_brightness",
                serde_json::json!({ "id": "all", "value": 0 })
            )),
            ReadResult::Command(Command::SetBrightness { display, value })
                if display == "all" && value == 0
        ));
    }

    #[test]
    fn rejects_set_brightness_with_missing_or_out_of_range_input() {
        for input in [
            serde_json::Value::Null,
            serde_json::json!({ "value": 45 }),
            serde_json::json!({ "id": "" }),
            serde_json::json!({ "id": "id-1" }),
            serde_json::json!({ "id": "id-1", "value": 101 }),
            serde_json::json!({ "id": "id-1", "value": -5 }),
            serde_json::json!({ "id": "id-1", "value": "45" }),
            serde_json::json!({ "id": "id-1", "value": 45.5 }),
        ] {
            assert!(
                matches!(
                    parse_request(&request("set_brightness", input.clone())),
                    ReadResult::Error(_)
                ),
                "input: {input}"
            );
        }
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
        current: StdMutex<BTreeMap<String, u8>>,
        source: BrightnessSource,
        calls: StdMutex<Vec<(String, u8)>>,
        steps: AtomicUsize,
        gets: AtomicUsize,
        warned: StdMutex<bool>,
        refuse_get: bool,
        selections: StdMutex<Vec<(String, BrightnessPolicy)>>,
    }

    impl FakeControl {
        fn new(displays: Vec<DisplayHandle>, current: u8, source: BrightnessSource) -> Self {
            let brightness = displays
                .iter()
                .map(|handle| (handle.id().to_string(), current))
                .collect();
            Self {
                displays,
                current: StdMutex::new(brightness),
                source,
                calls: StdMutex::new(Vec::new()),
                steps: AtomicUsize::new(0),
                gets: AtomicUsize::new(0),
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

        fn get_brightness(&self, handle: &DisplayHandle) -> Result<BrightnessState, MonitorError> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            if self.refuse_get {
                return Err(MonitorError::refused(
                    "brightness",
                    "control is off for this display",
                ));
            }
            Ok(BrightnessState {
                value: self
                    .current
                    .lock()
                    .unwrap()
                    .get(handle.id())
                    .copied()
                    .unwrap_or_default(),
                source: self.source,
            })
        }

        fn set_brightness(&self, handle: &DisplayHandle, value: u8) -> Result<(), MonitorError> {
            self.steps.fetch_add(1, Ordering::SeqCst);
            self.current
                .lock()
                .unwrap()
                .insert(handle.id().to_string(), value);
            self.calls
                .lock()
                .unwrap()
                .push((handle.id().to_string(), value));
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
        Runtime::new(
            control,
            store,
            Arc::new(NoLutProvider),
            |_title, _body| {},
            None,
            |_preferred| Ok(()),
        )
    }

    type Toasts = Arc<StdMutex<Vec<String>>>;

    fn runtime_with_toasts(
        control: Arc<FakeControl>,
        store: SessionStore,
    ) -> (Runtime<FakeControl>, Toasts) {
        let toasts: Toasts = Arc::new(StdMutex::new(Vec::new()));
        let sink = toasts.clone();
        let runtime = Runtime::new(
            control,
            store,
            Arc::new(NoLutProvider),
            move |_title, body| {
                sink.lock().unwrap().push(body.to_string());
            },
            None,
            |_preferred| Ok(()),
        );
        (runtime, toasts)
    }

    fn runtime_with_root(
        control: Arc<FakeControl>,
        store: SessionStore,
        config_root: Option<PathBuf>,
    ) -> Runtime<FakeControl> {
        Runtime::new(
            control,
            store,
            Arc::new(NoLutProvider),
            |_title, _body| {},
            config_root,
            |_preferred| Ok(()),
        )
    }

    fn preferred_root() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let config_root = dir.path().join("config").join("qol-tray");
        std::fs::create_dir_all(config_root.join("profile").join("default")).unwrap();
        (dir, config_root)
    }

    fn write_preferred(config_root: &std::path::Path, preferred: BTreeMap<String, u8>) {
        config::save_preferred(Some(config_root), &preferred).unwrap();
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
    fn hotkey_steps_every_connected_display_not_only_the_first() {
        let (_dir, store) = runtime_store();
        let control = Arc::new(FakeControl::new(
            vec![
                handle("id-1", "card0-DP-1"),
                handle("id-2", "card0-HDMI-1"),
                handle("id-3", "card0-DP-2"),
            ],
            60,
            BrightnessSource::Ddc,
        ));
        let (mut runtime, toasts) = runtime_with_toasts(control.clone(), store);
        runtime.handle(Command::Brightness {
            direction: -1,
            phase: Phase::Start,
        });
        assert_eq!(
            control.calls(),
            vec![
                ("id-1".to_string(), 55),
                ("id-2".to_string(), 55),
                ("id-3".to_string(), 55),
            ]
        );
        assert_eq!(
            toasts.lock().unwrap().as_slice(),
            ["Brightness down on 3 displays"]
        );
    }

    #[test]
    fn set_brightness_persists_preferred_to_the_device_file() {
        let (_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        let control = Arc::new(FakeControl::new(
            vec![handle("id-1", "card0-DP-1"), handle("id-2", "card0-HDMI-1")],
            60,
            BrightnessSource::Ddc,
        ));
        let saves = Arc::new(AtomicUsize::new(0));
        let saved = saves.clone();
        let saved_root = config_root.clone();
        let mut runtime = Runtime::new(
            control.clone(),
            store,
            Arc::new(NoLutProvider),
            |_title, _body| {},
            Some(config_root.clone()),
            move |preferred: &BTreeMap<String, u8>| {
                saved.fetch_add(1, Ordering::SeqCst);
                config::save_preferred(Some(&saved_root), preferred)
            },
        );
        runtime.handle(Command::SetBrightness {
            display: "id-2".into(),
            value: 25,
        });
        assert_eq!(control.calls(), vec![("id-2".to_string(), 25)]);
        assert_eq!(
            config::load_preferred(Some(&config_root)),
            BTreeMap::from([("id-2".to_string(), 25)]),
            "the daemon-owned preferred file carries exactly the written id"
        );
        assert_eq!(
            runtime.config().preferred_for("id-2"),
            None,
            "the tray-facing config never learns about preferred"
        );
        assert_eq!(saves.load(Ordering::SeqCst), 1);
        control.calls.lock().unwrap().clear();
        runtime.handle(Command::SetBrightness {
            display: "all".into(),
            value: 80,
        });
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 80), ("id-2".to_string(), 80)],
            "id all writes every connected display"
        );
        assert_eq!(
            config::load_preferred(Some(&config_root)),
            BTreeMap::from([("id-1".to_string(), 80), ("id-2".to_string(), 80)])
        );
        assert_eq!(saves.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn set_brightness_reports_an_unknown_display_instead_of_failing_silently() {
        let (_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        let control = Arc::new(FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            60,
            BrightnessSource::Ddc,
        ));
        let toasts: Toasts = Arc::new(StdMutex::new(Vec::new()));
        let sink = toasts.clone();
        let mut runtime = Runtime::new(
            control.clone(),
            store,
            Arc::new(NoLutProvider),
            move |_title, body| {
                sink.lock().unwrap().push(body.to_string());
            },
            Some(config_root.clone()),
            |_preferred| Ok(()),
        );
        runtime.handle(Command::SetBrightness {
            display: "id-gone".into(),
            value: 25,
        });
        assert_eq!(control.calls(), Vec::<(String, u8)>::new());
        assert_eq!(
            config::load_preferred_file(&config::preferred_path(&config_root).unwrap(), || {
                DeviceConfig::default()
            }),
            BTreeMap::new(),
            "an unknown display is never persisted"
        );
        assert_eq!(
            toasts.lock().unwrap().as_slice(),
            ["Brightness could not be set on the selected display"]
        );
    }

    #[test]
    fn start_restores_stale_snapshots_and_never_applies_preferred() {
        let (_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        write_preferred(&config_root, BTreeMap::from([("id-1".to_string(), 80)]));
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            60,
            BrightnessSource::Ddc,
        ));
        store
            .write_snapshot(&stale_snapshot("id-1", "card0-DP-1", 100, 60))
            .unwrap();
        let mut runtime = runtime_with_root(control.clone(), store, Some(config_root));
        runtime.start(&DeviceConfig::default());
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 100)],
            "crash restore runs and preferred is never written on top"
        );
        assert!(
            runtime
                .session()
                .store()
                .load_snapshot("id-1")
                .unwrap()
                .is_none(),
            "a recovered snapshot is cleared"
        );
    }

    #[test]
    fn exit_restores_after_preferred_and_is_idempotent() {
        let (_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        write_preferred(&config_root, BTreeMap::from([("id-1".to_string(), 80)]));
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with_root(control.clone(), store, Some(config_root));
        runtime.start(&DeviceConfig::default());
        assert!(runtime.handle(Command::ApplyPreferred));
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
        let (_root, config_root) = preferred_root();
        write_preferred(&config_root, BTreeMap::from([("id-1".to_string(), 20)]));
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            60,
            BrightnessSource::Ddc,
        ));
        store
            .write_snapshot(&stale_snapshot("id-1", "card0-DP-1", 50, 60))
            .unwrap();
        let mut runtime = runtime_with_root(control.clone(), store, Some(config_root));
        runtime.start(&DeviceConfig::default());
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 50)],
            "the crash-restored value stands; preferred is never layered on top"
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
    fn start_never_writes_brightness_when_preferred_is_configured() {
        let (_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        write_preferred(&config_root, BTreeMap::from([("id-1".to_string(), 30)]));
        let control = Arc::new(FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            75,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with_root(control.clone(), store, Some(config_root));
        runtime.start(&DeviceConfig::default());
        assert_eq!(
            control.calls(),
            Vec::<(String, u8)>::new(),
            "opening the settings panel spawns the daemon and must move no display"
        );
        assert_eq!(control.steps.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn apply_preferred_command_applies_every_configured_display() {
        let (_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        write_preferred(&config_root, BTreeMap::from([("id-1".to_string(), 85)]));
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            60,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with_root(control.clone(), store, Some(config_root));
        runtime.start(&DeviceConfig::default());
        control.calls.lock().unwrap().clear();
        assert!(runtime.handle(Command::ApplyPreferred));
        assert_eq!(control.calls(), vec![("id-1".to_string(), 85)]);
    }

    #[test]
    fn reload_config_applies_policy_and_never_preferred() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            60,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with(control.clone(), store);
        let initial = DeviceConfig {
            policy: BTreeMap::from([(
                "id-1".to_string(),
                PolicySelection {
                    policy: "ddc".into(),
                },
            )]),
            ..DeviceConfig::default()
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
            ..DeviceConfig::default()
        };
        assert_eq!(runtime.reload_config(&next), 1);
        assert_eq!(
            control.calls(),
            Vec::<(String, u8)>::new(),
            "preferred deltas from the tray config are ignored"
        );
        assert_eq!(control.selection("id-1"), BrightnessPolicy::Gamma);
        assert_eq!(runtime.config().preferred_for("id-1"), None);
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
            policy: BTreeMap::from([(
                "id-1".to_string(),
                PolicySelection {
                    policy: "off".into(),
                },
            )]),
            ..DeviceConfig::default()
        };
        runtime.start(&initial);
        control.calls.lock().unwrap().clear();
        assert_eq!(runtime.reload_config(&DeviceConfig::default()), 1);
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
        let preferred = BTreeMap::from([("id-1".to_string(), 80)]);
        let payload = displays_payload(&control, &preferred, &mut BTreeMap::new());
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
        let payload = displays_payload(&control, &BTreeMap::new(), &mut BTreeMap::new());
        assert_eq!(payload[0]["brightness"], serde_json::Value::Null);
        assert_eq!(payload[0]["source"], "unavailable");
        assert_eq!(payload[0]["detail"], "control is off for this display");
        assert_eq!(payload[0]["preferred"], serde_json::Value::Null);
    }

    #[test]
    fn status_payload_maps_no_displays_ok_and_unavailable() {
        let empty = FakeControl::new(Vec::new(), 0, BrightnessSource::Ddc);
        assert_eq!(
            status_payload(&empty, &mut BTreeMap::new()),
            serde_json::json!({ "state": "no_displays", "count": 0 })
        );
        let ok = FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            42,
            BrightnessSource::Ddc,
        );
        assert_eq!(
            status_payload(&ok, &mut BTreeMap::new()),
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
            status_payload(&refused, &mut BTreeMap::new()),
            serde_json::json!({ "state": "unavailable", "count": 1 })
        );
    }

    #[test]
    fn displays_payload_serves_the_written_value_without_a_hardware_read() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            60,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with(control.clone(), store);
        runtime.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 25,
        });
        let gets_after_write = control.gets.load(Ordering::SeqCst);
        let payload =
            displays_payload(&*control, &runtime.preferred, &mut runtime.brightness_cache);
        assert_eq!(payload[0]["brightness"], 25);
        assert_eq!(payload[0]["detail"], "25% via ddc");
        assert_eq!(
            control.gets.load(Ordering::SeqCst),
            gets_after_write,
            "a warm cache must not read the display hardware"
        );
    }

    #[test]
    fn displays_payload_reads_an_unknown_display_once_then_caches_it() {
        let control = FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            42,
            BrightnessSource::Ddc,
        );
        let mut cache = BTreeMap::new();
        let first = displays_payload(&control, &BTreeMap::new(), &mut cache);
        assert_eq!(first[0]["brightness"], 42);
        assert_eq!(control.gets.load(Ordering::SeqCst), 1);
        let second = displays_payload(&control, &BTreeMap::new(), &mut cache);
        assert_eq!(second[0]["brightness"], 42);
        assert_eq!(
            control.gets.load(Ordering::SeqCst),
            1,
            "the second poll is served from the cache"
        );
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn displays_payload_drops_cache_entries_for_gone_displays() {
        let control = FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            42,
            BrightnessSource::Ddc,
        );
        let mut cache = BTreeMap::from([(
            "id-gone".to_string(),
            BrightnessState {
                value: 50,
                source: BrightnessSource::Ddc,
            },
        )]);
        displays_payload(&control, &BTreeMap::new(), &mut cache);
        assert!(
            !cache.contains_key("id-gone"),
            "a display that left the topology must leave the cache"
        );
    }

    #[test]
    fn hotkey_steps_serve_the_cache_and_update_it_without_a_read() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            60,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with(control.clone(), store);
        runtime.handle(Command::Brightness {
            direction: 1,
            phase: Phase::Start,
        });
        assert_eq!(control.calls(), vec![("id-1".to_string(), 65)]);
        let gets_after_first_step = control.gets.load(Ordering::SeqCst);
        runtime.handle(Command::Brightness {
            direction: 1,
            phase: Phase::Start,
        });
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 65), ("id-1".to_string(), 70),]
        );
        assert_eq!(
            control.gets.load(Ordering::SeqCst),
            gets_after_first_step,
            "a warm cache steps without reading the hardware again"
        );
        assert_eq!(runtime.brightness_cache["id-1"].value, 70);
    }

    #[test]
    fn routes_queries_from_live_state_with_data() {
        let (_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        write_preferred(&config_root, BTreeMap::from([("id-1".to_string(), 80)]));
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            42,
            BrightnessSource::Ddc,
        ));
        let mut runtime: Runtime<dyn MonitorControl> = Runtime::new(
            control.clone(),
            store,
            Arc::new(NoLutProvider),
            |_title, _body| {},
            Some(config_root),
            |_preferred| Ok(()),
        );
        runtime.start(&DeviceConfig::default());
        set_live_state(Arc::new(Mutex::new(runtime)));

        let ReadResult::HandledWithData(payload) =
            parse_request(&request("displays", serde_json::Value::Null))
        else {
            panic!("displays must answer with data");
        };
        assert_eq!(payload[0]["connector"], "card0-DP-1");
        assert_eq!(
            payload[0]["brightness"], 42,
            "start reports the live value it read, never one it wrote"
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
    fn set_brightness_toasts_the_value_by_default() {
        let (_dir, store) = runtime_store();
        let control = Arc::new(FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            60,
            BrightnessSource::Ddc,
        ));
        let (mut runtime, toasts) = runtime_with_toasts(control.clone(), store);
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 25,
        });
        assert_eq!(
            toasts.lock().unwrap().as_slice(),
            ["Brightness 25%"],
            "the default config keeps the value toast"
        );
    }

    #[test]
    fn set_brightness_skips_the_value_toast_when_notify_on_change_is_off() {
        let (_dir, store) = runtime_store();
        let control = Arc::new(FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            60,
            BrightnessSource::Ddc,
        ));
        let (mut runtime, toasts) = runtime_with_toasts(control.clone(), store);
        runtime.start(&DeviceConfig {
            notify_on_change: false,
            ..DeviceConfig::default()
        });
        runtime.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 30,
        });
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 30)],
            "the write still happens"
        );
        assert!(
            toasts.lock().unwrap().is_empty(),
            "the value toast is silenced"
        );
        runtime.handle(Command::SetBrightness {
            display: "id-gone".into(),
            value: 35,
        });
        assert_eq!(
            toasts.lock().unwrap().as_slice(),
            ["Brightness could not be set on the selected display"],
            "error toasts stay unconditional"
        );
    }

    #[test]
    fn step_skips_the_value_toast_when_notify_on_change_is_off() {
        let (_dir, store) = runtime_store();
        let control = Arc::new(FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            60,
            BrightnessSource::Ddc,
        ));
        let (mut runtime, toasts) = runtime_with_toasts(control.clone(), store);
        runtime.start(&DeviceConfig {
            notify_on_change: false,
            ..DeviceConfig::default()
        });
        runtime.handle(Command::Brightness {
            direction: 1,
            phase: Phase::Start,
        });
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 65)],
            "stepping still works"
        );
        assert!(
            toasts.lock().unwrap().is_empty(),
            "the step toast is silenced"
        );
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
        let mut runtime = Runtime::new(
            control,
            store.clone(),
            Arc::new(NoLutProvider),
            notify,
            None,
            |_preferred| Ok(()),
        );
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
        let mut runtime = Runtime::new(
            control,
            store,
            Arc::new(NoLutProvider),
            notify,
            None,
            |_preferred| Ok(()),
        );
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
