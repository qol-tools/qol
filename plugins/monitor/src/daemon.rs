use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use qol_host_fixes::residency::HostResidency;
use qol_plugin_daemon::daemon::{self as core_daemon, DaemonConfig, ReadResult, SocketSource};
use qol_runtime::protocol::DaemonRequest;

use crate::config::{self, DeviceConfig};
use crate::host_night_light::{
    HostNightLight, HostNightLightStatus, NoopHostNightLight, TakeoverOutcome,
};
use crate::monitor::night::{
    self, Decision, Minute, NightState, Now, Reason, Schedule, ScheduleMode, Tint,
};
use crate::monitor::{
    BrightnessPolicy, BrightnessSource, BrightnessState, DisplayControl, GammaStateControl,
    MonitorError, BRIGHTNESS_MAX, BRIGHTNESS_MIN, BRIGHTNESS_STEP,
};
use crate::platform::MonitorControl;
use crate::session::{LutProvider, RestoreMode, Session, SessionStore, Snapshot};
use qol_windowing::display::DisplayHandle;

pub const HOLD_DEBOUNCE: Duration = Duration::from_millis(70);
pub const NIGHT_TICK: Duration = Duration::from_secs(30);
pub const HOST_NIGHT_LIGHT_SETTLE_SECS: i64 = 4;

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
    Night(NightRequest),
    Tick,
    Kill,
    Evicted,
    Handoff,
    HandoffSuccessor { generation: Option<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NightRequest {
    Toggle,
    On,
    Off,
}

pub fn parse_request(request: &DaemonRequest) -> ReadResult<Command> {
    match request.action.as_str() {
        "ping" => return ReadResult::Handled,
        "kill" => return ReadResult::Command(Command::Evicted),
        "settings" => return ReadResult::Command(Command::Settings),
        "apply" => return ReadResult::Command(Command::ApplyPreferred),
        "reload" => return ReadResult::Command(Command::Reload),
        "night_toggle" => return ReadResult::Command(Command::Night(NightRequest::Toggle)),
        "night_on" => return ReadResult::Command(Command::Night(NightRequest::On)),
        "night_off" => return ReadResult::Command(Command::Night(NightRequest::Off)),
        "handoff" => {
            let generation = request
                .input
                .get("generation")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string);
            return ReadResult::Command(Command::HandoffSuccessor { generation });
        }
        "set_brightness" => {
            return match parse_brightness_input(&request.input) {
                Ok((display, value)) => {
                    ReadResult::Command(Command::SetBrightness { display, value })
                }
                Err(error) => ReadResult::Error(error),
            };
        }
        "displays" | "status" | "night_mode" => return live_query(request.action.as_str()),
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
    let night_kelvin = runtime.active_night_kelvin();
    let payload = match name {
        "displays" => displays_payload(
            &*control,
            &preferred,
            &mut runtime.brightness_cache,
            night_kelvin,
        ),
        "status" => status_payload(&*control, &mut runtime.brightness_cache),
        "night_mode" => runtime.night_payload(),
        _ => unreachable!("live_query only handles declared queries"),
    };
    ReadResult::HandledWithData(payload)
}

fn policy_config(config: &DeviceConfig) -> DeviceConfig {
    let mut policy_only = config.clone();
    policy_only.preferred_brightness.clear();
    policy_only
}

fn local_now() -> Now {
    use chrono::Timelike;

    let now = chrono::Local::now();
    Now {
        unix: now.timestamp(),
        minute: Minute((now.hour() * 60 + now.minute()) as u16),
    }
}

pub(crate) fn displays_payload(
    control: &dyn MonitorControl,
    preferred: &BTreeMap<String, u8>,
    cache: &mut BTreeMap<String, BrightnessState>,
    night_kelvin: Option<u16>,
) -> serde_json::Value {
    let handles = control.enumerate().unwrap_or_default();
    let rows: Vec<serde_json::Value> = handles
        .iter()
        .map(|handle| {
            let id = handle.id().to_string();
            let policy = control.selection(handle.id()).label();
            let preferred = preferred.get(&id).copied();
            let (brightness, source, mut detail) = match cache.get(handle.id()).copied() {
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
            if let Some(kelvin) = night_kelvin {
                detail.push_str(&format!(" warm {kelvin}K"));
            }
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
    residency: ResidencyCheck,
    night: NightState,
    night_applied: Option<(bool, u16)>,
    night_tinted_displays: HashSet<String>,
    night_unsupported: bool,
    night_schedule_error: Option<String>,
    night_decision: Option<Decision>,
    night_next_change: Option<String>,
    night_settle_until_unix: Option<i64>,
    host_night_light_conflict: bool,
    host_night_light: Arc<dyn HostNightLight>,
    clock: Clock,
    night_platform_supported: bool,
}

type Notify = Arc<dyn Fn(&str, &str) + Send + Sync>;
type PreferredSave = Arc<dyn Fn(&BTreeMap<String, u8>) -> anyhow::Result<()> + Send + Sync>;
type ResidencyCheck = Arc<dyn Fn() -> bool + Send + Sync>;
type Clock = Arc<dyn Fn() -> Now + Send + Sync>;

impl<C: DisplayControl + GammaStateControl + ?Sized> Runtime<C> {
    pub fn new(
        control: Arc<C>,
        store: SessionStore,
        lut: Arc<dyn LutProvider>,
        notify: impl Fn(&str, &str) + Send + Sync + 'static,
        config_root: Option<PathBuf>,
        preferred_save: impl Fn(&BTreeMap<String, u8>) -> anyhow::Result<()> + Send + Sync + 'static,
        residency: impl Fn() -> bool + Send + Sync + 'static,
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
            residency: Arc::new(residency),
            night: NightState::default(),
            night_applied: None,
            night_tinted_displays: HashSet::new(),
            night_unsupported: false,
            night_schedule_error: None,
            night_decision: None,
            night_next_change: None,
            night_settle_until_unix: None,
            host_night_light_conflict: false,
            host_night_light: Arc::new(NoopHostNightLight),
            clock: Arc::new(local_now),
            night_platform_supported: true,
        }
    }

    pub fn session(&self) -> &Session<C> {
        &self.session
    }

    fn is_resident(&self) -> bool {
        (self.residency)()
    }

    fn with_host_night_light(mut self, host_night_light: Arc<dyn HostNightLight>) -> Self {
        self.host_night_light = host_night_light;
        self
    }

    fn with_night_platform_supported(mut self, supported: bool) -> Self {
        self.night_platform_supported = supported;
        self.night_unsupported = !supported;
        self
    }

    #[cfg(test)]
    fn with_adoption_generation(mut self, generation: Option<String>) -> Self {
        self.session = self.session.with_adoption_generation(generation);
        self
    }

    #[cfg(test)]
    fn with_residency(mut self, resident: bool) -> Self {
        self.residency = Arc::new(move || resident);
        self
    }

    #[cfg(test)]
    fn with_clock(mut self, clock: impl Fn() -> Now + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
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
        let handoffs = self.handoff_snapshots();
        let recovery = if self.is_resident() {
            crate::session::RestoreReport::default()
        } else {
            let recovery = self.session.restore_all(RestoreMode::Recovery);
            self.surface_gamma_warnings(&recovery);
            if let Err(error) = self.host_night_light.release(RestoreMode::Recovery) {
                self.host_night_light_conflict = true;
                eprintln!("[plugin-monitor] host night light recovery failed: {error}");
            }
            if recovery.restored > 0 {
                (self.notify)(
                    "Monitor",
                    &format!(
                        "Restored {} display{} after an unclean shutdown",
                        recovery.restored,
                        if recovery.restored == 1 { "" } else { "s" }
                    ),
                );
            }
            recovery
        };
        *self.config.lock().unwrap() = policy_config(config);
        self.preferred = config::load_preferred(self.config_root.as_deref());
        self.adopt_handoffs(&handoffs);
        self.apply_preferred_map();
        self.night = config::load_night_state(self.config_root.as_deref());
        self.evaluate_night(true);
        recovery
    }

    fn parsed_schedule(&self) -> (Schedule, Option<String>) {
        match self.config().night_schedule() {
            Ok(schedule) => (schedule, None),
            Err(error) => (
                Schedule {
                    mode: ScheduleMode::Off,
                    from: Minute(0),
                    to: Minute(0),
                },
                Some(error.to_string()),
            ),
        }
    }

    fn evaluate_night(&mut self, force: bool) {
        let config = self.config();
        let (schedule, schedule_error) = self.parsed_schedule();
        let now = (self.clock)();
        let decision = night::decide(&schedule, &self.night, now);
        let override_on = decision.active && decision.reason == Reason::Manual;
        let armed = schedule.mode == ScheduleMode::Daily || override_on;
        if armed {
            self.reconcile_host_night_light(true, now.unix);
        }
        let kelvin = config.night_kelvin();
        let target = (decision.active, kelvin);
        let previous = self.night_applied;
        let changed = previous != Some(target);
        let should_apply =
            decision.active || !decision.active && previous.is_some_and(|state| state.0);
        let settling = self
            .night_settle_until_unix
            .is_some_and(|until| now.unix < until);
        if !settling {
            let reassert = self.night_settle_until_unix.take().is_some();
            if should_apply {
                self.apply_night_tint(decision.active, kelvin, changed || force || reassert);
            } else if !decision.active {
                self.night_unsupported = !self.night_platform_supported;
            }
            self.night_applied = Some(target);
        }
        if !armed {
            self.reconcile_host_night_light(false, now.unix);
        }
        self.night_schedule_error = schedule_error;
        self.night_next_change = schedule.next_transition(now.minute).map(Minute::label);
        self.night_decision = Some(decision);
        if changed
            && previous.map(|state| state.0).unwrap_or(false) != decision.active
            && config.notify_on_change
        {
            let message = if decision.active {
                format!("Night mode on ({kelvin}K)")
            } else {
                "Night mode off".to_string()
            };
            (self.notify)("Monitor", &message);
        }
        trace_night(
            decision,
            kelvin,
            self.host_night_light.status(),
            self.host_night_light_conflict,
            changed || force,
        );
    }

    fn apply_night_tint(&mut self, active: bool, kelvin: u16, reset_targets: bool) {
        if !self.night_platform_supported {
            self.night_unsupported = true;
            self.night_tinted_displays.clear();
            return;
        }
        let handles = self.session.control().enumerate().unwrap_or_default();
        let handle_count = handles.len();
        let targets: Vec<DisplayHandle> = if active && !reset_targets {
            handles
                .into_iter()
                .filter(|handle| !self.night_tinted_displays.contains(handle.id()))
                .collect()
        } else {
            handles
        };
        let tint = if active {
            Tint::from_kelvin(kelvin)
        } else {
            Tint::NEUTRAL
        };
        let mut successes = 0;
        let mut unsupported = 0;
        for handle in &targets {
            match self.session.mutate_tint(handle, tint) {
                Ok(()) => {
                    successes += 1;
                    if active {
                        self.night_tinted_displays.insert(handle.id().to_string());
                    }
                }
                Err(MonitorError::Unsupported { .. }) => unsupported += 1,
                Err(error) => {
                    eprintln!(
                        "[plugin-monitor] night mode write failed on {}: {error}",
                        handle.connector()
                    );
                }
            }
        }
        if !active {
            self.night_tinted_displays.clear();
        }
        if active && (reset_targets || !targets.is_empty()) {
            self.night_unsupported = handle_count == 0
                || (!targets.is_empty() && successes == 0 && unsupported == targets.len());
        }
    }

    fn reconcile_host_night_light(&mut self, armed: bool, now_unix: i64) {
        let result = if armed && !self.host_night_light.is_taken_over() {
            self.host_night_light.take_over().map(|outcome| {
                if outcome == TakeoverOutcome::Disabled {
                    self.night_settle_until_unix = Some(now_unix + HOST_NIGHT_LIGHT_SETTLE_SECS);
                }
            })
        } else if !armed && self.host_night_light.is_taken_over() {
            self.night_settle_until_unix = None;
            self.host_night_light.release(RestoreMode::Exit)
        } else {
            Ok(())
        };
        match result {
            Ok(()) => self.host_night_light_conflict = false,
            Err(error) => {
                self.host_night_light_conflict = armed;
                eprintln!("[plugin-monitor] host night light takeover failed: {error}");
            }
        }
    }

    fn active_night_kelvin(&self) -> Option<u16> {
        self.night_decision
            .filter(|decision| decision.active && !self.night_tinted_displays.is_empty())
            .map(|_| self.config().night_kelvin())
    }

    fn night_payload(&self) -> serde_json::Value {
        let decision = self.night_decision.unwrap_or(Decision {
            active: false,
            reason: Reason::Off,
            next_change_unix: None,
        });
        let state = if self.night_schedule_error.is_some() {
            "invalid_schedule"
        } else if self.host_night_light_conflict {
            "conflict"
        } else if self.night_unsupported {
            "unsupported"
        } else if decision.active {
            "active"
        } else {
            "inactive"
        };
        serde_json::json!({
            "state": state,
            "active": decision.active,
            "temperature": self.config().night_kelvin(),
            "reason": decision.reason.label(),
            "next_change": self.night_next_change,
            "host_night_light": self.host_night_light.status().label(),
        })
    }

    fn next_night_wait(&self) -> Option<Duration> {
        if let Some(until) = self.night_settle_until_unix {
            let remaining = (until - (self.clock)().unix).max(1);
            return Some(Duration::from_secs(remaining as u64).min(NIGHT_TICK));
        }
        let active = self
            .night_decision
            .map(|decision| decision.active)
            .unwrap_or(false);
        let scheduled = self.config().night_schedule().ok().is_some_and(|schedule| {
            schedule.mode == ScheduleMode::Daily && schedule.from != schedule.to
        });
        (active || scheduled).then_some(NIGHT_TICK)
    }

    fn handoff_snapshots(&self) -> Vec<Snapshot> {
        let Ok(inventory) = self.session.store().load_all() else {
            return Vec::new();
        };
        inventory
            .snapshots
            .into_iter()
            .filter(|snapshot| snapshot.handoff)
            .collect()
    }

    fn adopt_handoffs(&mut self, handoffs: &[Snapshot]) {
        if handoffs.is_empty() {
            return;
        }
        let Ok(handles) = self.session.control().enumerate() else {
            return;
        };
        for snapshot in handoffs {
            if !self.session.handoff_is_for_this_generation(snapshot) {
                continue;
            }
            let Some(handle) = handles
                .iter()
                .find(|handle| handle.id() == snapshot.display_id)
                .or_else(|| {
                    handles
                        .iter()
                        .find(|handle| handle.connector() == snapshot.connector)
                })
            else {
                continue;
            };
            let handle = handle.clone();
            self.session.adopt(&handle);
            self.remember_brightness(&handle, snapshot.last_value);
            let _ = self.session.store().write_snapshot(&Snapshot {
                handoff: false,
                ..snapshot.clone()
            });
        }
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
                self.night = NightState::default();
                if let Err(error) =
                    config::save_night_state(self.config_root.as_deref(), &self.night)
                {
                    eprintln!("[plugin-monitor] failed to clear night mode override: {error:#}");
                }
                self.evaluate_night(true);
                true
            }
            Command::Night(request) => {
                let (schedule, _) = self.parsed_schedule();
                let now = (self.clock)();
                self.night = match request {
                    NightRequest::Toggle => night::toggled(&schedule, &self.night, now),
                    NightRequest::On => night::set_active(&schedule, &self.night, now, true),
                    NightRequest::Off => night::set_active(&schedule, &self.night, now, false),
                };
                if let Err(error) =
                    config::save_night_state(self.config_root.as_deref(), &self.night)
                {
                    eprintln!("[plugin-monitor] failed to persist night mode state: {error:#}");
                }
                self.evaluate_night(false);
                true
            }
            Command::Tick => {
                self.evaluate_night(false);
                true
            }
            Command::Kill => {
                if !self.is_resident() {
                    let report = self.session.restore_all(RestoreMode::Exit);
                    self.surface_gamma_warnings(&report);
                    if let Err(error) = self.host_night_light.release(RestoreMode::Exit) {
                        eprintln!("[plugin-monitor] host night light restore failed: {error}");
                    }
                }
                false
            }
            Command::Evicted => {
                self.session
                    .mark_handoff_all(Some(crate::session::EVICTION_GENERATION));
                self.host_night_light
                    .mark_handoff(Some(crate::session::EVICTION_GENERATION));
                false
            }
            Command::Handoff => {
                self.session.mark_handoff_all(None);
                self.host_night_light.mark_handoff(None);
                false
            }
            Command::HandoffSuccessor { generation } => {
                self.session.mark_handoff_all(generation.as_deref());
                self.host_night_light.mark_handoff(generation.as_deref());
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

fn receive_commands(
    rx: &Receiver<Command>,
    timeout: Option<Duration>,
) -> Result<Option<Command>, ()> {
    let Some(timeout) = timeout else {
        return rx.recv().map(Some).map_err(|_| ());
    };
    match rx.recv_timeout(timeout) {
        Ok(command) => Ok(Some(command)),
        Err(RecvTimeoutError::Timeout) => Ok(Some(Command::Tick)),
        Err(RecvTimeoutError::Disconnected) => Err(()),
    }
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
    loop {
        let timeout = runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .next_night_wait();
        let Ok(Some(command)) = receive_commands(rx, timeout) else {
            break;
        };
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

fn install_signal_handlers(tx: Sender<Command>) -> signal_hook::iterator::Handle {
    let mut signals = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ])
    .expect("failed to register the SIGTERM and SIGHUP handlers");
    let handle = signals.handle();
    std::thread::Builder::new()
        .name("monitor-signals".into())
        .spawn(move || {
            for signal in signals.forever() {
                let command = match signal {
                    signal_hook::consts::SIGTERM => Some(Command::Kill),
                    signal_hook::consts::SIGHUP => Some(Command::Handoff),
                    _ => None,
                };
                if let Some(command) = command {
                    let _ = tx.send(command);
                    return;
                }
            }
        })
        .expect("failed to spawn the signal forwarder");
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
    let _sigterm = install_signal_handlers(tx);
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
    let host_night_light = crate::host_night_light::control(config_root.as_deref());
    Runtime::new(
        control,
        store,
        lut,
        notify,
        config_root,
        preferred_save,
        || HostResidency::current().is_resident(),
    )
    .with_host_night_light(host_night_light)
    .with_night_platform_supported(cfg!(target_os = "linux"))
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

fn trace_night(
    decision: Decision,
    kelvin: u16,
    host: HostNightLightStatus,
    conflict: bool,
    emitted: bool,
) {
    #[cfg(debug_assertions)]
    if emitted {
        qol_runtime::probe!(
            "MONITOR_SESSION",
            "event=night active={} reason={} kelvin={} next={:?} host={} conflict={}",
            decision.active,
            decision.reason.label(),
            kelvin,
            decision.next_change_unix,
            host.label(),
            conflict
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = (decision, kelvin, host, conflict, emitted);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{self, BrightnessPreference, PolicySelection};
    use crate::host_night_light::HostNightLightError;
    use crate::monitor::{
        BrightnessPolicy, BrightnessSource, DisplayCapabilities, DisplayMode, GammaState,
        GammaTable, HdrState, RestoreOutcome,
    };
    use crate::session::NoLutProvider;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;

    static SIGNAL_TEST_LOCK: StdMutex<()> = StdMutex::new(());

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
            ReadResult::Command(Command::Evicted)
        ));
        assert!(matches!(
            parse_request(&request("nope", serde_json::Value::Null)),
            ReadResult::Fallback
        ));
    }

    #[test]
    fn routes_handoff_with_and_without_a_delivered_successor_generation() {
        assert!(matches!(
            parse_request(&request(
                "handoff",
                serde_json::json!({ "generation": "successor-gen" })
            )),
            ReadResult::Command(Command::HandoffSuccessor {
                generation: Some(value)
            }) if value == "successor-gen"
        ));
        assert!(matches!(
            parse_request(&request("handoff", serde_json::Value::Null)),
            ReadResult::Command(Command::HandoffSuccessor { generation: None })
        ));
    }

    #[test]
    fn routes_every_night_mode_action() {
        assert!(matches!(
            parse_request(&request("night_toggle", serde_json::Value::Null)),
            ReadResult::Command(Command::Night(NightRequest::Toggle))
        ));
        assert!(matches!(
            parse_request(&request("night_on", serde_json::Value::Null)),
            ReadResult::Command(Command::Night(NightRequest::On))
        ));
        assert!(matches!(
            parse_request(&request("night_off", serde_json::Value::Null)),
            ReadResult::Command(Command::Night(NightRequest::Off))
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
        tints: StdMutex<Vec<(String, Tint)>>,
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
                tints: StdMutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, u8)> {
            self.calls.lock().unwrap().clone()
        }

        fn tints(&self) -> Vec<(String, Tint)> {
            self.tints.lock().unwrap().clone()
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

        fn set_tint(&self, handle: &DisplayHandle, tint: Tint) -> Result<(), MonitorError> {
            self.tints
                .lock()
                .unwrap()
                .push((handle.id().to_string(), tint));
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
            || false,
        )
    }

    struct StaticLut;

    impl LutProvider for StaticLut {
        fn capture(&self, _connector: &str) -> Option<GammaTable> {
            Some(GammaTable {
                red: vec![0, u16::MAX],
                green: vec![0, u16::MAX],
                blue: vec![0, u16::MAX],
            })
        }

        fn write_guarded(
            &self,
            _handle: &DisplayHandle,
            _original: &GammaTable,
            _last_value: u8,
            _last_tint: Tint,
        ) -> crate::session::LutRestoreOutcome {
            crate::session::LutRestoreOutcome::Restored
        }

        fn adopt_baseline(
            &self,
            _handle: &DisplayHandle,
            _original: &GammaTable,
            _last_value: u8,
            _last_tint: Tint,
        ) {
        }
    }

    fn night_runtime(
        control: Arc<FakeControl>,
        store: SessionStore,
        config_root: PathBuf,
        clock: Arc<StdMutex<Now>>,
    ) -> Runtime<FakeControl> {
        Runtime::new(
            control,
            store,
            Arc::new(StaticLut),
            |_title, _body| {},
            Some(config_root),
            |_preferred| Ok(()),
            || false,
        )
        .with_clock(move || *clock.lock().unwrap())
    }

    fn runtime_with_generation(
        control: Arc<FakeControl>,
        store: SessionStore,
        generation: &str,
    ) -> Runtime<FakeControl> {
        runtime_with(control, store).with_adoption_generation(Some(generation.to_string()))
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
            || false,
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
            || false,
        )
    }

    fn runtime_with_residency(
        control: Arc<FakeControl>,
        store: SessionStore,
        resident: bool,
    ) -> Runtime<FakeControl> {
        Runtime::new(
            control,
            store,
            Arc::new(NoLutProvider),
            |_title, _body| {},
            None,
            |_preferred| Ok(()),
            move || resident,
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
            last_tint: crate::monitor::night::Tint::NEUTRAL,
            mutations: 3,
            clean: false,
            handoff: false,
            adopt_generation: None,
            lut: None,
            checksum: String::new(),
        }
    }

    #[test]
    fn night_toggle_tints_every_display_persists_and_then_restores_neutral() {
        let (_session_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        let displays = vec![handle("id-1", "card0-DP-1"), handle("id-2", "card0-HDMI-1")];
        let control = Arc::new(FakeControl::new(displays, 70, BrightnessSource::Ddc));
        let clock = Arc::new(StdMutex::new(Now {
            unix: 100_000,
            minute: Minute(12 * 60),
        }));
        let mut runtime = night_runtime(control.clone(), store, config_root.clone(), clock);
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::Night(NightRequest::Toggle));
        let warm = Tint::from_kelvin(3500);
        assert_eq!(
            control.tints(),
            vec![("id-1".to_string(), warm), ("id-2".to_string(), warm)]
        );
        assert_eq!(
            config::load_night_state(Some(&config_root)).override_active,
            Some(true)
        );
        assert_eq!(runtime.night_payload()["state"], "active");
        let rows = displays_payload(
            &*control,
            &BTreeMap::new(),
            &mut BTreeMap::new(),
            runtime.active_night_kelvin(),
        );
        assert!(rows[0]["detail"].as_str().unwrap().contains("warm 3500K"));

        runtime.handle(Command::Night(NightRequest::Toggle));
        assert_eq!(control.tints().len(), 4);
        assert_eq!(control.tints()[2].1, Tint::NEUTRAL);
        assert_eq!(control.tints()[3].1, Tint::NEUTRAL);
        assert_eq!(runtime.night_payload()["state"], "inactive");
    }

    #[derive(Default)]
    struct DisablingNightLight {
        taken: StdMutex<bool>,
    }

    impl HostNightLight for DisablingNightLight {
        fn take_over(&self) -> Result<TakeoverOutcome, HostNightLightError> {
            *self.taken.lock().unwrap() = true;
            Ok(TakeoverOutcome::Disabled)
        }

        fn release(&self, _mode: RestoreMode) -> Result<(), HostNightLightError> {
            *self.taken.lock().unwrap() = false;
            Ok(())
        }

        fn mark_handoff(&self, _successor: Option<&str>) {}

        fn is_taken_over(&self) -> bool {
            *self.taken.lock().unwrap()
        }

        fn status(&self) -> HostNightLightStatus {
            if self.is_taken_over() {
                HostNightLightStatus::TakenOver
            } else {
                HostNightLightStatus::Off
            }
        }
    }

    #[test]
    fn tint_waits_for_the_host_night_light_fade_before_touching_the_ramp() {
        let (_session_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        let control = Arc::new(FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            70,
            BrightnessSource::Ddc,
        ));
        let clock = Arc::new(StdMutex::new(Now {
            unix: 100_000,
            minute: Minute(12 * 60),
        }));
        let mut runtime = night_runtime(control.clone(), store, config_root, clock.clone())
            .with_host_night_light(Arc::new(DisablingNightLight::default()));
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::Night(NightRequest::On));
        assert!(
            control.tints().is_empty(),
            "the host night light is still fading out; writing now would capture its ramp"
        );
        assert_eq!(runtime.night_payload()["state"], "active");
        assert_eq!(
            runtime.next_night_wait(),
            Some(Duration::from_secs(HOST_NIGHT_LIGHT_SETTLE_SECS as u64))
        );
        clock.lock().unwrap().unix += 1;
        runtime.handle(Command::Tick);
        assert!(control.tints().is_empty());
        clock.lock().unwrap().unix += HOST_NIGHT_LIGHT_SETTLE_SECS;
        runtime.handle(Command::Tick);
        assert_eq!(
            control.tints(),
            vec![("id-1".to_string(), Tint::from_kelvin(3500))]
        );
        assert_eq!(runtime.next_night_wait(), Some(NIGHT_TICK));
        runtime.handle(Command::Tick);
        assert_eq!(
            control.tints().len(),
            1,
            "a steady-state tick must not rewrite the gamma ramp"
        );
        runtime.handle(Command::Night(NightRequest::Off));
        assert_eq!(control.tints().len(), 2);
        assert_eq!(control.tints()[1].1, Tint::NEUTRAL);
        assert_eq!(runtime.next_night_wait(), None);
    }

    #[test]
    fn daily_tick_changes_only_at_the_schedule_boundary() {
        let (_session_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        let control = Arc::new(FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            70,
            BrightnessSource::Ddc,
        ));
        let clock = Arc::new(StdMutex::new(Now {
            unix: 100_000,
            minute: Minute(21 * 60),
        }));
        let mut runtime = night_runtime(control.clone(), store, config_root, clock.clone());
        let config = DeviceConfig {
            night_schedule: "daily".to_string(),
            ..DeviceConfig::default()
        };
        runtime.start(&config);
        assert_eq!(control.tints().len(), 1);
        runtime.handle(Command::Tick);
        assert_eq!(
            control.tints().len(),
            1,
            "a steady-state tick must not rewrite the gamma ramp"
        );
        *clock.lock().unwrap() = Now {
            unix: 136_000,
            minute: Minute(7 * 60),
        };
        runtime.handle(Command::Tick);
        assert_eq!(control.tints().len(), 2);
        assert_eq!(control.tints()[1].1, Tint::NEUTRAL);
        assert_eq!(runtime.night_payload()["reason"], "schedule");
        assert_eq!(runtime.night_payload()["state"], "inactive");
    }

    #[test]
    fn unsupported_gamma_surfaces_without_panicking() {
        let (_session_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        let control = Arc::new(FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            70,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with_root(control.clone(), store, Some(config_root));
        runtime.start(&DeviceConfig::default());
        assert!(runtime.handle(Command::Night(NightRequest::On)));
        assert!(control.tints().is_empty());
        assert_eq!(runtime.night_payload()["state"], "unsupported");
        assert_eq!(runtime.night_payload()["active"], true);
    }

    #[test]
    fn invalid_schedule_is_visible_in_the_live_payload() {
        let (_session_dir, store) = runtime_store();
        let control = Arc::new(FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            70,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with(control, store);
        runtime.start(&DeviceConfig {
            night_from: "24:00".to_string(),
            ..DeviceConfig::default()
        });
        assert_eq!(runtime.night_payload()["state"], "invalid_schedule");
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
            || false,
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
            || false,
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
    fn portable_start_restores_the_stale_baseline_then_applies_preferred() {
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
        let mut runtime =
            runtime_with_root(control.clone(), store, Some(config_root)).with_residency(false);
        runtime.start(&DeviceConfig::default());
        assert_eq!(
            control.calls(),
            vec![
                ("id-1".to_string(), 100),
                ("id-1".to_string(), 80),
            ],
            "the crash restore returns the baseline first, then preferred is applied on top (portable)"
        );
        let snapshot = runtime
            .session()
            .store()
            .load_snapshot("id-1")
            .unwrap()
            .expect("portable start captures a fresh baseline");
        assert_eq!(
            snapshot.value, 100,
            "the restored baseline becomes the live untouched-host baseline"
        );
    }

    #[test]
    fn resident_start_keeps_the_baseline_and_applies_preferred() {
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
        let mut runtime =
            runtime_with_root(control.clone(), store, Some(config_root)).with_residency(true);
        runtime.start(&DeviceConfig::default());
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 80)],
            "resident start never restores the old baseline and converges to preferred instead"
        );
        let snapshot = runtime
            .session()
            .store()
            .load_snapshot("id-1")
            .unwrap()
            .expect("apply_preferred captures a fresh snapshot");
        assert_eq!(
            snapshot.value, 60,
            "the live value becomes the baseline so it can be restored if residency is ever disabled"
        );
    }

    #[test]
    fn resident_start_adopts_handoffs_then_applies_preferred() {
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
            .write_snapshot(&crate::session::Snapshot {
                handoff: true,
                adopt_generation: Some("reload-1".to_string()),
                last_value: 60,
                ..stale_snapshot("id-1", "card0-DP-1", 100, 60)
            })
            .unwrap();
        let mut runtime = runtime_with_root(control.clone(), store, Some(config_root))
            .with_residency(true)
            .with_adoption_generation(Some("reload-1".to_string()));
        runtime.start(&DeviceConfig::default());
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 80)],
            "the resident successor adopts the handoff without restoring and then applies preferred"
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
        let mut runtime =
            runtime_with_root(control.clone(), store, Some(config_root)).with_residency(false);
        runtime.start(&DeviceConfig::default());
        assert!(runtime.handle(Command::ApplyPreferred));
        control.calls.lock().unwrap().clear();
        assert!(!runtime.handle(Command::Kill), "kill stops the loop");
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 100)],
            "portable exit restore returns to the pre-daemon state"
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
    fn resident_exit_preserves_the_display_and_keeps_the_baseline_snapshot() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with_residency(control.clone(), store.clone(), true);
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 60,
        });
        control.calls.lock().unwrap().clear();
        assert!(!runtime.handle(Command::Kill), "kill stops the loop");
        assert_eq!(
            control.calls(),
            Vec::<(String, u8)>::new(),
            "resident exit must never write the display"
        );
        let after_exit = control.current.lock().unwrap().get("id-1").copied();
        assert_eq!(
            after_exit,
            Some(60),
            "resident exit keeps the display exactly as the user set it"
        );
        let snapshot = store
            .load_snapshot("id-1")
            .unwrap()
            .expect("a resident exit keeps the baseline snapshot on disk");
        assert_eq!(
            snapshot.value, 100,
            "the untouched-host baseline survives so disabling residency later can restore it"
        );
    }

    #[test]
    fn resident_then_portable_flip_takes_effect_at_the_next_decision_point() {
        let (_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        write_preferred(&config_root, BTreeMap::from([("id-1".to_string(), 80)]));
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let residen = std::sync::Arc::new(StdMutex::new(true));
        let gate = residen.clone();
        let mut runtime = Runtime::new(
            control.clone(),
            store,
            Arc::new(NoLutProvider),
            |_title, _body| {},
            Some(config_root),
            |_preferred| Ok(()),
            || false,
        );
        runtime.residency = Arc::new(move || *gate.lock().unwrap());
        runtime.start(&DeviceConfig::default());
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 80)],
            "starting resident converges to preferred"
        );
        control.calls.lock().unwrap().clear();
        *residen.lock().unwrap() = false;
        runtime.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 60,
        });
        control.calls.lock().unwrap().clear();
        assert!(
            !runtime.handle(Command::Kill),
            "the flip to portable is read at exit, not cached at daemon start"
        );
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 100)],
            "the now-portable host restores the baseline at the next decision point"
        );
    }

    #[test]
    fn restart_after_clean_exit_writes_nothing_and_keeps_the_baseline() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut first = runtime_with(control.clone(), store.clone());
        first.start(&DeviceConfig::default());
        first.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 60,
        });
        assert!(!first.handle(Command::Kill));
        let after_exit = control.current.lock().unwrap().get("id-1").copied();
        assert_eq!(after_exit, Some(100), "exit restore returns the baseline");
        drop(first);
        control.calls.lock().unwrap().clear();
        let mut second = runtime_with(control.clone(), store.clone());
        second.start(&DeviceConfig::default());
        assert_eq!(
            control.calls(),
            Vec::<(String, u8)>::new(),
            "a restart after a clean exit must not write the display"
        );
        let after_restart = control.current.lock().unwrap().get("id-1").copied();
        assert_eq!(
            after_restart,
            Some(100),
            "the display keeps whatever the user left on it"
        );
        assert!(
            store.load_snapshot("id-1").unwrap().is_none(),
            "the clean snapshot is retired without being re-applied"
        );
        assert!(!second.handle(Command::Kill));
        assert_eq!(
            control.calls(),
            Vec::<(String, u8)>::new(),
            "nothing to restore on the next exit either"
        );
    }

    #[test]
    fn crash_recovery_restores_the_baseline_and_surfaces_the_write() {
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
        let toasts: Toasts = Arc::new(StdMutex::new(Vec::new()));
        let sink = toasts.clone();
        let mut runtime = Runtime::new(
            control.clone(),
            store,
            Arc::new(NoLutProvider),
            move |_title, body| {
                sink.lock().unwrap().push(body.to_string());
            },
            None,
            |_preferred| Ok(()),
            || false,
        );
        runtime.start(&DeviceConfig::default());
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 100)],
            "an unclean crash restores the pre-qol baseline at the next start"
        );
        assert_eq!(
            toasts.lock().unwrap().as_slice(),
            ["Restored 1 display after an unclean shutdown"],
            "the crash restore must be surfaced, never silent"
        );
    }

    #[test]
    fn reload_handoff_keeps_the_display_value_and_writes_no_hardware() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut first = runtime_with_generation(control.clone(), store.clone(), "reload-1");
        first.start(&DeviceConfig::default());
        first.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 60,
        });
        control.calls.lock().unwrap().clear();
        assert!(
            !first.handle(Command::Handoff),
            "a reload must end the daemon loop"
        );
        let after_handoff = control.current.lock().unwrap().get("id-1").copied();
        assert_eq!(
            after_handoff,
            Some(60),
            "a reload must not move the display at exit"
        );
        assert!(
            control.calls().is_empty(),
            "the reload exit must not write the hardware"
        );
        drop(first);
        let mut second = runtime_with_generation(control.clone(), store.clone(), "reload-1");
        second.start(&DeviceConfig::default());
        let after_restart = control.current.lock().unwrap().get("id-1").copied();
        assert_eq!(
            after_restart,
            Some(60),
            "a reloaded daemon must not move the display at start"
        );
        assert!(
            control.calls().is_empty(),
            "the reloaded start must not write the hardware"
        );
        let snapshot = store.load_snapshot("id-1").unwrap().unwrap();
        assert_eq!(
            snapshot.value, 100,
            "the pre-qol baseline survives a reload"
        );
        assert_eq!(snapshot.last_value, 60);
        assert!(
            !snapshot.clean,
            "the reload must not mark a clean exit that was never restored"
        );
        assert!(
            !snapshot.handoff,
            "the successor must clear the handoff marker after adopting"
        );
        assert!(!second.handle(Command::Kill));
        let after_exit = control.current.lock().unwrap().get("id-1").copied();
        assert_eq!(
            after_exit,
            Some(100),
            "a real exit after a reload still restores the baseline"
        );
    }

    #[test]
    fn reload_successor_adopts_unaddressed_handoff_with_zero_hardware_writes() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut predecessor = runtime_with_generation(control.clone(), store.clone(), "reload-1");
        predecessor.start(&DeviceConfig::default());
        predecessor.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 60,
        });
        control.calls.lock().unwrap().clear();
        assert!(
            !predecessor.handle(Command::Handoff),
            "SIGHUP ends the reload loop"
        );
        drop(predecessor);

        let marked = store.load_snapshot("id-1").unwrap().unwrap();
        assert!(marked.handoff, "SIGHUP marks the handoff");

        let mut successor = runtime_with_generation(control.clone(), store.clone(), "reload-1");
        let report = successor.start(&DeviceConfig::default());
        assert!(
            control.calls().is_empty(),
            "the reload successor must adopt without writing hardware: {:?}",
            control.calls()
        );
        assert_eq!(report.restored, 0);
        let after = control.current.lock().unwrap().get("id-1").copied();
        assert_eq!(
            after,
            Some(60),
            "the display stays where the user set it across the reload"
        );
        let adopted = store.load_snapshot("id-1").unwrap().unwrap();
        assert!(
            !adopted.handoff,
            "the successor clears the handoff marker after adopting"
        );
    }

    #[test]
    fn handoff_stamps_the_successor_generation_not_the_predecessors_own() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut first = runtime_with_generation(control.clone(), store.clone(), "predecessor-gen");
        first.start(&DeviceConfig::default());
        first.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 60,
        });
        assert!(
            !first.handle(Command::HandoffSuccessor {
                generation: Some("successor-gen".into()),
            }),
            "the orchestrator-delivered handoff ends the loop"
        );
        drop(first);

        let marked = store.load_snapshot("id-1").unwrap().unwrap();
        assert!(marked.handoff, "the handoff marks the snapshot");
        assert_eq!(
            marked.adopt_generation.as_deref(),
            Some("successor-gen"),
            "the handoff stamps the SUCCESSOR generation id the orchestrator delivered, not the predecessor's own {}",
            "predecessor-gen"
        );
    }

    #[test]
    fn a_successor_of_a_different_generation_does_not_adopt_an_addressed_handoff() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut first = runtime_with_generation(control.clone(), store.clone(), "gen-reload-a");
        first.start(&DeviceConfig::default());
        first.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 60,
        });
        control.calls.lock().unwrap().clear();
        assert!(
            !first.handle(Command::HandoffSuccessor {
                generation: Some("gen-reload-a".into()),
            }),
            "the addressed handoff ends the loop"
        );
        drop(first);

        let mut different = runtime_with_generation(control.clone(), store.clone(), "gen-reload-b");
        let report = different.start(&DeviceConfig::default());
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 100)],
            "a different generation is a genuine orphan of this handoff and must restore the baseline"
        );
        assert_eq!(report.restored, 1);
        assert!(
            store.load_snapshot("id-1").unwrap().is_none(),
            "the restored handoff snapshot must be cleared"
        );
    }

    #[test]
    fn a_promoted_daemon_without_identity_does_not_adopt_an_orphaned_handoff() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut first = runtime_with_generation(control.clone(), store.clone(), "gen-reload-a");
        first.start(&DeviceConfig::default());
        first.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 60,
        });
        control.calls.lock().unwrap().clear();
        assert!(!first.handle(Command::Handoff), "SIGHUP ends the loop");
        drop(first);

        let mut promoted_later = runtime_with(control.clone(), store.clone());
        let report = promoted_later.start(&DeviceConfig::default());
        assert_eq!(
            report.restored, 1,
            "a daemon that boots with no generation identity (long after promotion) must restore the orphaned baseline"
        );
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 100)],
            "the orphan's recovery writes the pre-qol baseline back"
        );
        assert!(
            store.load_snapshot("id-1").unwrap().is_none(),
            "the restored handoff snapshot must be cleared"
        );
    }

    #[test]
    fn the_same_generation_reload_successor_adopts_with_zero_hardware_writes() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut predecessor =
            runtime_with_generation(control.clone(), store.clone(), "gen-reload-a");
        predecessor.start(&DeviceConfig::default());
        predecessor.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 60,
        });
        control.calls.lock().unwrap().clear();
        assert!(
            !predecessor.handle(Command::Handoff),
            "SIGHUP ends the reload loop"
        );
        drop(predecessor);

        let mut successor = runtime_with_generation(control.clone(), store.clone(), "gen-reload-a");
        let report = successor.start(&DeviceConfig::default());
        assert!(
            control.calls().is_empty(),
            "the reload successor must adopt without writing hardware: {:?}",
            control.calls()
        );
        assert_eq!(report.restored, 0);
        let adopted = store.load_snapshot("id-1").unwrap().unwrap();
        assert!(!adopted.handoff, "the successor clears the handoff marker");
    }

    #[test]
    fn a_reload_successor_whose_handoff_was_addressed_by_the_orchestrator_adopts_across_a_rebuild()
    {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut predecessor = runtime_with_generation(control.clone(), store.clone(), "digest-old");
        predecessor.start(&DeviceConfig::default());
        predecessor.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 60,
        });
        control.calls.lock().unwrap().clear();
        assert!(
            !predecessor.handle(Command::Handoff),
            "SIGHUP ends the reload loop"
        );
        drop(predecessor);

        let mut stamped = store.load_snapshot("id-1").unwrap().unwrap();
        stamped.adopt_generation = Some("digest-new".to_string());
        store.write_snapshot(&stamped).unwrap();

        let mut successor = runtime_with_generation(control.clone(), store.clone(), "digest-new");
        let report = successor.start(&DeviceConfig::default());
        assert!(
            control.calls().is_empty(),
            "the orchestrator-stamped successor adopts even when the build digest changed: {:?}",
            control.calls()
        );
        assert_eq!(report.restored, 0);
    }

    #[test]
    fn cold_start_after_an_aborted_reload_restores_and_notifies() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut predecessor = runtime_with(control.clone(), store.clone());
        predecessor.start(&DeviceConfig::default());
        predecessor.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 60,
        });
        control.calls.lock().unwrap().clear();
        assert!(
            !predecessor.handle(Command::Handoff),
            "SIGHUP ends the reload loop"
        );
        drop(predecessor);

        let (mut cold_start, toasts) = runtime_with_toasts(control.clone(), store.clone());
        let report = cold_start.start(&DeviceConfig::default());
        assert_eq!(
            report.restored, 1,
            "a pure-stable cold start must restore the pre-qol baseline"
        );
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 100)],
            "the orphan's recovery writes the baseline back"
        );
        {
            let bodies = toasts.lock().unwrap();
            assert!(
                bodies.iter().any(|body| body.contains("Restored")),
                "a permanent mutation must never be silent: {bodies:?}"
            );
        }
        assert!(
            store.load_snapshot("id-1").unwrap().is_none(),
            "the restored handoff snapshot must be cleared"
        );
    }

    #[test]
    fn kill_after_handoff_without_adoption_still_restores_the_baseline() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut runtime = runtime_with(control.clone(), store.clone());
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 60,
        });
        assert!(!runtime.handle(Command::Handoff));
        drop(runtime);
        let mut successor = runtime_with(control.clone(), store.clone());
        assert!(!successor.handle(Command::Kill));
        let after_exit = control.current.lock().unwrap().get("id-1").copied();
        assert_eq!(
            after_exit,
            Some(100),
            "a real quit during the handoff window still restores the baseline"
        );
    }

    #[test]
    fn stale_handoff_without_a_successor_is_restored_on_the_next_recovery() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut first = runtime_with_generation(control.clone(), store.clone(), "gen-a");
        first.start(&DeviceConfig::default());
        first.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 60,
        });
        control.calls.lock().unwrap().clear();
        assert!(!first.handle(Command::Handoff), "SIGHUP ends the loop");
        drop(first);

        let snap = store.load_snapshot("id-1").unwrap().unwrap();
        assert!(snap.handoff, "SIGHUP must mark the snapshot for handoff");
        assert_eq!(
            snap.adopt_generation.as_deref(),
            None,
            "a bare SIGHUP with no orchestrator successor id leaves the handoff unaddressed"
        );

        store.write_snapshot(&snap).unwrap();

        let mut cold_start = runtime_with(control.clone(), store.clone());
        let report = cold_start.start(&DeviceConfig::default());
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 100)],
            "a handoff whose successor never started must restore the baseline when an unrelated cold start boots"
        );
        assert_eq!(report.restored, 1);
        assert!(
            store.load_snapshot("id-1").unwrap().is_none(),
            "the restored handoff snapshot must be cleared"
        );
    }

    #[test]
    fn a_successor_that_starts_late_still_adopts_instead_of_restoring() {
        let (_dir, store) = runtime_store();
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            100,
            BrightnessSource::Ddc,
        ));
        let mut predecessor = runtime_with_generation(control.clone(), store.clone(), "gen-s");
        predecessor.start(&DeviceConfig::default());
        predecessor.handle(Command::SetBrightness {
            display: "id-1".into(),
            value: 60,
        });
        control.calls.lock().unwrap().clear();
        assert!(
            !predecessor.handle(Command::Handoff),
            "SIGHUP ends the loop"
        );
        drop(predecessor);

        let snap = store.load_snapshot("id-1").unwrap().unwrap();
        assert!(snap.handoff, "SIGHUP must mark the snapshot for handoff");
        assert_eq!(
            snap.adopt_generation.as_deref(),
            None,
            "a bare SIGHUP with no orchestrator successor id leaves the handoff unaddressed"
        );

        store.write_snapshot(&snap).unwrap();

        let mut successor = runtime_with_generation(control.clone(), store.clone(), "gen-s");
        let report = successor.start(&DeviceConfig::default());
        assert_eq!(
            control.calls(),
            Vec::<(String, u8)>::new(),
            "the intended successor adopts no matter how much time passed: booting must never write hardware"
        );
        assert_eq!(report.restored, 0);
        assert!(
            store.load_snapshot("id-1").unwrap().is_some(),
            "an adopted handoff is kept so the successor owns the display"
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
        let mut runtime =
            runtime_with_root(control.clone(), store, Some(config_root)).with_residency(false);
        runtime.start(&DeviceConfig::default());
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 50), ("id-1".to_string(), 20)],
            "the crash-restored value is written before preferred is layered on top"
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
    fn start_applies_preferred_in_both_residency_modes() {
        for resident in [false, true] {
            let (_dir, store) = runtime_store();
            let (_root, config_root) = preferred_root();
            write_preferred(&config_root, BTreeMap::from([("id-1".to_string(), 30)]));
            let control = Arc::new(FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                75,
                BrightnessSource::Ddc,
            ));
            let mut runtime = runtime_with_root(control.clone(), store, Some(config_root))
                .with_residency(resident);
            runtime.start(&DeviceConfig::default());
            assert_eq!(
                control.calls(),
                vec![("id-1".to_string(), 30)],
                "start applies preferred whether resident={resident}"
            );
            assert_eq!(control.steps.load(Ordering::SeqCst), 1);
        }
    }

    #[test]
    fn start_applies_a_differing_preferred_over_a_settled_snapshot() {
        let (_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        write_preferred(&config_root, BTreeMap::from([("id-1".to_string(), 100)]));
        let display = handle("id-1", "card0-DP-1");
        let control = Arc::new(FakeControl::new(
            vec![display.clone()],
            50,
            BrightnessSource::Ddc,
        ));
        let settled = Snapshot {
            value: 50,
            last_value: 100,
            mutations: 2,
            clean: true,
            ..stale_snapshot("id-1", "card0-DP-1", 100, 50)
        };
        store.write_snapshot(&settled).unwrap();
        let mut runtime = runtime_with_root(control.clone(), store, Some(config_root));
        runtime.start(&DeviceConfig::default());
        assert_eq!(
            control.calls(),
            vec![("id-1".to_string(), 100)],
            "a clean settled snapshot is dropped and preferred is applied at start"
        );
        let snapshot = runtime
            .session()
            .store()
            .load_snapshot("id-1")
            .unwrap()
            .unwrap();
        assert_eq!(
            snapshot.value, 50,
            "the on-disk host baseline stays the live pre-profile value"
        );
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
        let payload = displays_payload(&control, &preferred, &mut BTreeMap::new(), None);
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
        let payload = displays_payload(&control, &BTreeMap::new(), &mut BTreeMap::new(), None);
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
        let payload = displays_payload(
            &*control,
            &runtime.preferred,
            &mut runtime.brightness_cache,
            None,
        );
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
        let first = displays_payload(&control, &BTreeMap::new(), &mut cache, None);
        assert_eq!(first[0]["brightness"], 42);
        assert_eq!(control.gets.load(Ordering::SeqCst), 1);
        let second = displays_payload(&control, &BTreeMap::new(), &mut cache, None);
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
        displays_payload(&control, &BTreeMap::new(), &mut cache, None);
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
            || false,
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
            payload[0]["brightness"], 80,
            "start applies the preferred value in both residency modes"
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
        let _guard = SIGNAL_TEST_LOCK.lock().unwrap();
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
        let _sigterm = install_signal_handlers(tx);
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
    fn sighup_handoff_exits_without_restoring_and_marks_the_snapshot() {
        let _guard = SIGNAL_TEST_LOCK.lock().unwrap();
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
        let _sighup = install_signal_handlers(tx);
        let loop_thread = std::thread::spawn(move || run_loop(&runtime, &rx));
        let status = std::process::Command::new("kill")
            .args(["-HUP", &std::process::id().to_string()])
            .status()
            .expect("kill must run");
        assert!(status.success());
        loop_thread.join().expect("the loop must exit after SIGHUP");
        let after = control.current.lock().unwrap().get("id-1").copied();
        assert_eq!(
            after,
            Some(60),
            "SIGHUP must leave the display exactly as the user set it"
        );
        assert!(
            control.calls().is_empty(),
            "SIGHUP must not write the hardware"
        );
        let snap = store.load_snapshot("id-1").unwrap().unwrap();
        assert!(snap.handoff, "SIGHUP must mark the reload handoff");
        assert_eq!(snap.value, 100, "the baseline survives the handoff");
        assert_eq!(snap.last_value, 60);
        assert!(!snap.clean);
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
            || false,
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
            || false,
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
