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
use crate::monitor::layout::{
    layout_rows, mode_lists, mode_rows, placements_from_snapshots, resolve_arrange,
    resolve_config_layout, resolve_mode, snapshot_for, ArrangeRequest,
};
use crate::monitor::night::{
    self, Decision, Minute, NightState, Now, Reason, Schedule, ScheduleMode, Tint,
};
use crate::monitor::{
    BrightnessState, DisplayControl, DisplayMode, GammaStateControl, MonitorError, BRIGHTNESS_MAX,
    BRIGHTNESS_MIN, BRIGHTNESS_STEP,
};
use crate::platform::MonitorControl;
use crate::session::{
    LayoutSnapshot, LutProvider, ModeRecord, PlacementRecord, RestoreMode, Session, SessionStore,
    Snapshot,
};
use qol_windowing::display::{DisplayHandle, DisplayPlacement, DisplaySnapshot};

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

#[derive(Debug, Clone)]
pub enum Command {
    Brightness {
        direction: i8,
        phase: Phase,
    },
    SetBrightness {
        display: String,
        value: u8,
    },
    SetMode {
        display: String,
        token: Option<u64>,
        width: u32,
        height: u32,
        refresh: Option<u32>,
    },
    SetPrimary {
        display: String,
    },
    Arrange {
        placements: Vec<ArrangeRequest>,
        primary: Option<String>,
    },
    ApplyLayout,
    Settings,
    ApplyPreferred,
    Reload,
    Night(NightRequest),
    Tick,
    Kill,
    Evicted,
    Handoff,
    HandoffSuccessor {
        generation: Option<String>,
    },
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
        "set_mode" => {
            return match parse_set_mode_input(&request.input) {
                Ok(parsed) => ReadResult::Command(Command::SetMode {
                    display: parsed.display,
                    token: parsed.token,
                    width: parsed.width,
                    height: parsed.height,
                    refresh: parsed.refresh,
                }),
                Err(error) => ReadResult::Error(error),
            };
        }
        "set_primary" => {
            return match parse_display_input("set_primary", &request.input) {
                Ok(display) => ReadResult::Command(Command::SetPrimary { display }),
                Err(error) => ReadResult::Error(error),
            };
        }
        "arrange" => {
            return match parse_arrange_input(&request.input) {
                Ok((placements, primary)) => ReadResult::Command(Command::Arrange {
                    placements,
                    primary,
                }),
                Err(error) => ReadResult::Error(error),
            };
        }
        "apply_layout" => return ReadResult::Command(Command::ApplyLayout),
        "displays" | "status" | "night_mode" | "layout" | "modes" => {
            return live_query(request.action.as_str())
        }
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

fn parse_display_input(action: &str, input: &serde_json::Value) -> Result<String, String> {
    input
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{action} input requires a display id"))
}

struct SetModeInput {
    display: String,
    token: Option<u64>,
    width: u32,
    height: u32,
    refresh: Option<u32>,
}

fn parse_set_mode_input(input: &serde_json::Value) -> Result<SetModeInput, String> {
    let display = parse_display_input("set_mode", input)?;
    let token = match input.get("token") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .ok_or_else(|| "set_mode token must be a positive integer".to_string())?,
        ),
    };
    let width = input
        .get("width")
        .and_then(serde_json::Value::as_u64)
        .and_then(|raw| u32::try_from(raw).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| "set_mode input requires a positive width".to_string())?;
    let height = input
        .get("height")
        .and_then(serde_json::Value::as_u64)
        .and_then(|raw| u32::try_from(raw).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| "set_mode input requires a positive height".to_string())?;
    let refresh = match input.get("refresh") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            value
                .as_u64()
                .and_then(|raw| u32::try_from(raw).ok())
                .filter(|hz| *hz > 0)
                .ok_or_else(|| "set_mode refresh must be a positive integer".to_string())?,
        ),
    };
    Ok(SetModeInput {
        display,
        token,
        width,
        height,
        refresh,
    })
}

fn parse_arrange_input(
    input: &serde_json::Value,
) -> Result<(Vec<ArrangeRequest>, Option<String>), String> {
    let placements: Vec<ArrangeRequest> = match input.get("placements") {
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| format!("arrange placements are invalid: {error}"))?,
        None => return Err("arrange input requires placements".to_string()),
    };
    if placements.is_empty() {
        return Err("arrange input requires at least one placement".to_string());
    }
    let primary = match input.get("primary") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            value
                .as_str()
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .ok_or_else(|| "arrange primary must be a display id".to_string())?,
        ),
    };
    Ok((placements, primary))
}

fn live_query(name: &str) -> ReadResult<Command> {
    let Some(live) = live_state() else {
        return ReadResult::Error("monitor daemon state is not ready".into());
    };
    live_query_from(live, name)
}

fn live_query_from(
    live: &Arc<Mutex<Runtime<dyn MonitorControl>>>,
    name: &str,
) -> ReadResult<Command> {
    let (control, preferred, night_kelvin, night_payload, mut cache) = {
        let Ok(runtime) = live.lock() else {
            return ReadResult::Error("monitor daemon state is poisoned".into());
        };
        (
            Arc::clone(runtime.session().control()),
            runtime.preferred.clone(),
            runtime.active_night_kelvin(),
            runtime.night_payload(),
            runtime.session.brightness_cache(),
        )
    };
    let payload = match name {
        "displays" => displays_payload(&*control, &preferred, &mut cache, night_kelvin),
        "status" => status_payload(&*control, &mut cache),
        "night_mode" => night_payload,
        "layout" => layout_payload(&*control),
        "modes" => modes_payload(&*control),
        _ => unreachable!("live_query only handles declared queries"),
    };
    if matches!(name, "displays" | "status") {
        if let Ok(runtime) = live.lock() {
            runtime.session.merge_brightness_cache(&cache);
        }
    }
    ReadResult::HandledWithData(payload)
}

fn layout_payload(control: &dyn MonitorControl) -> serde_json::Value {
    match control.snapshot() {
        Ok(snapshots) => serde_json::json!(layout_rows(&snapshots)),
        Err(_) => serde_json::json!([]),
    }
}

fn modes_payload(control: &dyn MonitorControl) -> serde_json::Value {
    let Ok(snapshots) = control.snapshot() else {
        return serde_json::json!([]);
    };
    let modes = mode_lists(&snapshots, |handle| control.list_modes(handle));
    let writable = snapshots.iter().all(|snapshot| {
        control
            .probe(&snapshot.handle)
            .map(|capabilities| capabilities.modes)
            .unwrap_or(false)
    });
    serde_json::json!(mode_rows(&snapshots, &modes, writable))
}

fn resolve_requested_mode(
    modes: &[DisplayMode],
    token: Option<u64>,
    width: u32,
    height: u32,
    refresh: Option<u32>,
) -> Result<DisplayMode, MonitorError> {
    let Some(token) = token else {
        return resolve_mode(modes, width, height, refresh);
    };
    modes
        .iter()
        .find(|mode| mode.token == token)
        .cloned()
        .ok_or_else(|| {
            let available = if modes.is_empty() {
                "none".to_string()
            } else {
                modes
                    .iter()
                    .map(|mode| format!("{}x{}@{}", mode.width, mode.height, mode.refresh_hz))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            MonitorError::refused(
                "modes",
                format!("no mode matches token {token}; available: {available}"),
            )
        })
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
    let snapshots = control.snapshot().unwrap_or_default();
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
            if let Some(snapshot) = snapshots
                .iter()
                .find(|snapshot| snapshot.handle.id() == handle.id())
                .or_else(|| {
                    snapshots
                        .iter()
                        .find(|snapshot| snapshot.handle.connector() == handle.connector())
                })
            {
                detail.push_str(&format!(
                    " {:+}{:+}",
                    snapshot.bounds.x.round() as i32,
                    snapshot.bounds.y.round() as i32
                ));
                if let Some(mode) = &snapshot.mode {
                    detail.push_str(&format!(
                        " {}x{}@{}Hz",
                        mode.width, mode.height, mode.refresh_hz
                    ));
                }
                if snapshot.primary {
                    detail.push_str(" primary");
                }
            }
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
    residency: ResidencyCheck,
    night: NightState,
    night_applied: Option<(bool, u16)>,
    night_unsupported: bool,
    night_schedule_error: Option<String>,
    night_decision: Option<Decision>,
    night_next_change: Option<String>,
    night_settle_until_unix: Option<i64>,
    host_night_light_conflict: bool,
    host_night_light: Arc<dyn HostNightLight>,
    clock: Clock,
    night_apply_error: Option<String>,
    night_native: bool,
    night_fallback_reason: Option<String>,
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
            residency: Arc::new(residency),
            night: NightState::default(),
            night_applied: None,
            night_unsupported: false,
            night_schedule_error: None,
            night_decision: None,
            night_next_change: None,
            night_settle_until_unix: None,
            host_night_light_conflict: false,
            host_night_light: Arc::new(NoopHostNightLight),
            clock: Arc::new(local_now),
            night_apply_error: None,
            night_native: false,
            night_fallback_reason: None,
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
    fn restore_layout(&self, mode: RestoreMode) {
        let report = self.session.restore_layout(mode);
        trace_layout_restore(&report);
        if report.failed > 0 {
            (self.notify)(
                "Monitor",
                "Display layout could not be restored; the saved layout is kept",
            );
        }
    }

    pub fn start(&mut self, config: &DeviceConfig) -> crate::session::RestoreReport {
        let handoffs = self.startup_snapshots();
        let recovery = if self.is_resident() {
            crate::session::RestoreReport::default()
        } else {
            self.restore_layout(RestoreMode::Recovery);
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
        self.adopt_startup_snapshots(&handoffs);
        self.session.adopt_layout_handoff();
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
        let kelvin = config.night_kelvin();
        let gamma_brightness = self.session.uses_gamma_brightness();
        let native_compatible = !gamma_brightness;
        let target = (decision.active, kelvin);
        let previous = self.night_applied;
        let changed = previous != Some(target);
        if (armed || self.night_native)
            && native_compatible
            && self.host_night_light.native_supported()
            && !self.session.tinted_displays().is_empty()
        {
            self.apply_night_tint(false, kelvin, true);
            if !self.session.tinted_displays().is_empty() {
                return;
            }
        }
        let native = if !native_compatible {
            self.night_fallback_reason =
                Some("Software brightness and night tint share one gamma ramp".into());
            if self.night_native {
                match self.host_night_light.take_over() {
                    Ok(TakeoverOutcome::Disabled) => {
                        self.night_settle_until_unix =
                            Some(now.unix + HOST_NIGHT_LIGHT_SETTLE_SECS);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        self.night_apply_error = Some(error.to_string());
                        return;
                    }
                }
            }
            false
        } else if (armed || self.night_native)
            && (changed || force || self.night_apply_error.is_some())
        {
            match self.host_night_light.apply_native(decision.active, kelvin) {
                Ok(applied) => {
                    self.night_apply_error = None;
                    if applied {
                        self.host_night_light_conflict = false;
                        self.night_fallback_reason = None;
                    }
                    applied
                }
                Err(crate::host_night_light::HostNightLightError::Unsupported(reason)) => {
                    self.night_fallback_reason = Some(reason);
                    false
                }
                Err(error) => {
                    self.night_apply_error = Some(error.to_string());
                    true
                }
            }
        } else {
            self.night_native
        };
        self.night_native = native;
        if armed && !native {
            self.reconcile_host_night_light(true, now.unix);
        }
        let should_apply = decision.active
            || previous.is_some_and(|state| state.0)
            || !self.session.tinted_displays().is_empty();
        let settling = self
            .night_settle_until_unix
            .is_some_and(|until| now.unix < until);
        if !settling {
            let reassert = self.night_settle_until_unix.take().is_some();
            if native {
                self.night_unsupported = false;
            } else if should_apply {
                self.apply_night_tint(decision.active, kelvin, changed || force || reassert);
            } else if !decision.active {
                self.night_unsupported = false;
            }
            self.night_applied = Some(target);
        }
        if !armed && !gamma_brightness {
            self.reconcile_host_night_light(false, now.unix);
            if !self.host_night_light.is_taken_over() {
                self.night_native = false;
            }
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
        qol_runtime::probe!(
            "MONITOR_SESSION",
            "event=night_strategy native={} strategy={} error={:?}",
            self.night_native,
            self.host_night_light.strategy(),
            self.night_apply_error
        );
    }

    fn apply_night_tint(&mut self, active: bool, kelvin: u16, reset_targets: bool) {
        let handles = match self.session.control().enumerate() {
            Ok(handles) => handles,
            Err(error) => {
                self.night_apply_error = Some(error.to_string());
                return;
            }
        };
        let handle_count = handles.len();
        let targets: Vec<DisplayHandle> = if active && !reset_targets {
            handles
                .into_iter()
                .filter(|handle| !self.session.tinted_displays().contains(handle.id()))
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
        self.night_apply_error = None;
        for handle in &targets {
            match self.session.mutate_tint(handle, tint) {
                Ok(()) => {
                    successes += 1;
                }
                Err(error @ MonitorError::Unsupported { .. }) => {
                    unsupported += 1;
                    self.night_apply_error = Some(error.to_string());
                }
                Err(error) => {
                    self.night_apply_error = Some(error.to_string());
                    eprintln!(
                        "[plugin-monitor] night mode write failed on {}: {error}",
                        handle.connector()
                    );
                }
            }
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
                if !armed {
                    self.night_apply_error = Some(error.to_string());
                }
                eprintln!("[plugin-monitor] host night light takeover failed: {error}");
            }
        }
    }

    fn active_night_kelvin(&self) -> Option<u16> {
        self.night_decision
            .filter(|decision| {
                decision.active && (self.night_native || !self.session.tinted_displays().is_empty())
            })
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
        } else if self.night_apply_error.is_some() {
            "failed"
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
            "error": self.night_apply_error,
            "fallback_reason": self.night_fallback_reason,
            "strategy": if self.night_native { self.host_night_light.strategy() } else { "gamma" },
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
        (active
            || scheduled
            || self.night_apply_error.is_some()
            || !self.session.tinted_displays().is_empty())
        .then(|| {
            self.night_decision
                .and_then(|decision| decision.next_change_unix)
                .map(|until| {
                    Duration::from_secs((until - (self.clock)().unix).max(1) as u64).min(NIGHT_TICK)
                })
                .unwrap_or(NIGHT_TICK)
        })
    }

    fn startup_snapshots(&self) -> Vec<Snapshot> {
        let Ok(inventory) = self.session.store().load_all() else {
            return Vec::new();
        };
        inventory
            .snapshots
            .into_iter()
            .filter(|snapshot| {
                !snapshot.clean
                    && (snapshot.handoff || (self.is_resident() && snapshot.lut.is_some()))
            })
            .collect()
    }

    fn adopt_startup_snapshots(&mut self, handoffs: &[Snapshot]) {
        if handoffs.is_empty() {
            return;
        }
        let Ok(handles) = self.session.control().enumerate() else {
            return;
        };
        for snapshot in handoffs {
            if !(snapshot.handoff && self.session.handoff_is_for_this_generation(snapshot))
                && !(self.is_resident() && snapshot.lut.is_some())
            {
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
            if !self.session.adopt(&handle) {
                continue;
            }
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
                applied += 1;
            }
        }
        applied
    }

    fn layout_snapshot(&self, snapshots: &[DisplaySnapshot]) -> LayoutSnapshot {
        LayoutSnapshot {
            schema_version: crate::session::LAYOUT_SCHEMA_VERSION,
            layout_id: crate::session::LAYOUT_SNAPSHOT_ID.to_string(),
            placements: snapshots
                .iter()
                .map(|snapshot| PlacementRecord {
                    id: snapshot.handle.id().to_string(),
                    connector: snapshot.handle.connector().to_string(),
                    x: snapshot.bounds.x.round() as i32,
                    y: snapshot.bounds.y.round() as i32,
                    primary: snapshot.primary,
                })
                .collect(),
            modes: snapshots
                .iter()
                .filter_map(|snapshot| {
                    snapshot.mode.as_ref().map(|mode| ModeRecord {
                        id: snapshot.handle.id().to_string(),
                        connector: snapshot.handle.connector().to_string(),
                        token: mode.token,
                        width: mode.width,
                        height: mode.height,
                        refresh_hz: mode.refresh_hz,
                    })
                })
                .collect(),
            mutations: 0,
            handoff: false,
            adopt_generation: None,
        }
    }

    fn display_snapshots(&self, failure_prefix: &str) -> Option<Vec<DisplaySnapshot>> {
        match self.session.control().snapshot() {
            Ok(snapshots) => Some(snapshots),
            Err(error) => {
                (self.notify)(
                    "Monitor",
                    &format!("{failure_prefix}: display state is unavailable: {error}"),
                );
                None
            }
        }
    }

    fn claim_layout(&self, snapshots: &[DisplaySnapshot], failure_prefix: &str) -> bool {
        let capture = self.layout_snapshot(snapshots);
        match self
            .session
            .store()
            .claim_layout_in_topology(&capture, snapshots)
        {
            Ok(()) => true,
            Err(error) => {
                (self.notify)(
                    "Monitor",
                    &format!(
                        "{failure_prefix}: the restore snapshot could not be written: {error:#}"
                    ),
                );
                false
            }
        }
    }

    fn record_layout_mutation(&self, failure_prefix: &str) -> bool {
        match self.session.store().touch_layout() {
            Ok(()) => true,
            Err(error) => {
                (self.notify)(
                    "Monitor",
                    &format!(
                        "{failure_prefix}: the restore snapshot could not be updated: {error:#}"
                    ),
                );
                false
            }
        }
    }

    fn reassert_gamma(&self, handles: &[DisplayHandle]) -> crate::session::RestoreReport {
        let mut report = crate::session::RestoreReport::default();
        for handle in handles {
            report.record(self.session.reassert_gamma(handle));
        }
        trace_gamma_reassert(&report);
        self.surface_gamma_warnings(&report);
        report
    }

    fn reapply_display_state(&mut self, op: &str) {
        let handles = match self.session.control().enumerate() {
            Ok(handles) => handles,
            Err(error) => {
                eprintln!(
                    "[plugin-monitor] display state reapply could not enumerate displays: {error}"
                );
                Vec::new()
            }
        };
        let report = self.reassert_gamma(&handles);
        self.evaluate_night(true);
        let night_active = self
            .night_decision
            .map(|decision| decision.active)
            .unwrap_or(false);
        qol_runtime::probe!(
            "MONITOR_SESSION",
            "event=display_reassert op={} displays={} restored={} failed={} night_active={}",
            op,
            handles.len(),
            report.restored,
            report.failed,
            night_active
        );
    }

    fn apply_placements(
        &self,
        snapshots: &[DisplaySnapshot],
        placements: &[DisplayPlacement],
        failure_prefix: &str,
        op: &str,
        display: &str,
    ) -> bool {
        if !self.claim_layout(snapshots, failure_prefix) {
            return false;
        }
        if !self.record_layout_mutation(failure_prefix) {
            return false;
        }
        if let Err(error) = self.session.control().set_layout(placements) {
            qol_runtime::probe!(
                "MONITOR_DISPLAY_WRITE",
                "event=rejected op={} display={} error=\"{}\"",
                op,
                qol_runtime::probe::token(display),
                qol_runtime::probe::quoted(&error.to_string(), 160)
            );
            (self.notify)("Monitor", &format!("{failure_prefix}: {error}"));
            return false;
        }
        true
    }

    fn apply_mode(
        &self,
        display: &str,
        token: Option<u64>,
        width: u32,
        height: u32,
        refresh: Option<u32>,
    ) -> bool {
        let Some(snapshots) = self.display_snapshots("Mode not set") else {
            return false;
        };
        let target = match snapshot_for(&snapshots, display) {
            Ok(target) => target,
            Err(error) => {
                (self.notify)("Monitor", &format!("Mode not set: {error}"));
                return false;
            }
        };
        let modes = match self.session.control().list_modes(&target.handle) {
            Ok(modes) => modes,
            Err(error) => {
                (self.notify)("Monitor", &format!("Mode not set: {error}"));
                return false;
            }
        };
        let mode = match resolve_requested_mode(&modes, token, width, height, refresh) {
            Ok(mode) => mode,
            Err(error) => {
                (self.notify)("Monitor", &format!("Mode not set: {error}"));
                return false;
            }
        };
        if !self.claim_layout(&snapshots, "Mode not set") {
            return false;
        }
        if !self.record_layout_mutation("Mode not set") {
            return false;
        }
        if let Err(error) = self.session.control().set_mode(&target.handle, &mode) {
            qol_runtime::probe!(
                "MONITOR_DISPLAY_WRITE",
                "event=rejected op=mode display={} error=\"{}\"",
                qol_runtime::probe::token(target.handle.id()),
                qol_runtime::probe::quoted(&error.to_string(), 160)
            );
            (self.notify)("Monitor", &format!("Mode not set: {error}"));
            return false;
        }
        if self.config().notify_on_change {
            (self.notify)(
                "Monitor",
                &format!("Mode {}x{}@{}Hz", mode.width, mode.height, mode.refresh_hz),
            );
        }
        true
    }

    fn apply_primary(&self, display: &str) -> bool {
        let Some(snapshots) = self.display_snapshots("Primary not set") else {
            return false;
        };
        let target = match snapshot_for(&snapshots, display) {
            Ok(target) => target,
            Err(error) => {
                (self.notify)("Monitor", &format!("Primary not set: {error}"));
                return false;
            }
        };
        let mut placements = placements_from_snapshots(&snapshots);
        for placement in &mut placements {
            placement.primary = placement.handle.id() == target.handle.id();
        }
        let applied = self.apply_placements(
            &snapshots,
            &placements,
            "Primary not set",
            "primary",
            target.handle.id(),
        );
        if applied && self.config().notify_on_change {
            (self.notify)(
                "Monitor",
                &format!("Primary display is now {}", target.handle.connector()),
            );
        }
        applied
    }

    fn apply_arrange(&self, requested: &[ArrangeRequest], primary: Option<&str>) -> bool {
        let Some(snapshots) = self.display_snapshots("Layout not applied") else {
            return false;
        };
        let placements = match resolve_arrange(&snapshots, requested, primary) {
            Ok(placements) => placements,
            Err(error) => {
                (self.notify)("Monitor", &format!("Layout not applied: {error}"));
                return false;
            }
        };
        let applied = self.apply_placements(
            &snapshots,
            &placements,
            "Layout not applied",
            "layout",
            "none",
        );
        if applied && self.config().notify_on_change {
            (self.notify)(
                "Monitor",
                &format!(
                    "Arranged {} display{}",
                    placements.len(),
                    if placements.len() == 1 { "" } else { "s" }
                ),
            );
        }
        applied
    }

    fn apply_config_layout(&self) -> bool {
        let layout = self.config().layout_position;
        if layout.is_empty() {
            return false;
        }
        let Some(snapshots) = self.display_snapshots("Layout not applied") else {
            return false;
        };
        let placements = match resolve_config_layout(&snapshots, &layout) {
            Ok(placements) => placements,
            Err(error) => {
                (self.notify)("Monitor", &format!("Layout not applied: {error}"));
                return false;
            }
        };
        let applied = self.apply_placements(
            &snapshots,
            &placements,
            "Layout not applied",
            "layout",
            "none",
        );
        if applied && self.config().notify_on_change {
            (self.notify)(
                "Monitor",
                &format!(
                    "Applied the configured layout to {} display{}",
                    placements.len(),
                    if placements.len() == 1 { "" } else { "s" }
                ),
            );
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
            Command::SetMode {
                display,
                token,
                width,
                height,
                refresh,
            } => {
                if self.apply_mode(&display, token, width, height, refresh) {
                    self.reapply_display_state("mode");
                }
                true
            }
            Command::SetPrimary { display } => {
                if self.apply_primary(&display) {
                    self.reapply_display_state("primary");
                }
                true
            }
            Command::Arrange {
                placements,
                primary,
            } => {
                if self.apply_arrange(&placements, primary.as_deref()) {
                    self.reapply_display_state("layout");
                }
                true
            }
            Command::ApplyLayout => {
                if self.apply_config_layout() {
                    self.reapply_display_state("apply-layout");
                }
                true
            }
            Command::Settings => {
                if let Err(error) = qol_apps::desktop_integration::open_plugin_settings_via_tray(
                    crate::hotkeys::PLUGIN_ID,
                ) {
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
                    self.restore_layout(RestoreMode::Exit);
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

    fn cached_brightness(&self, handle: &DisplayHandle) -> Option<BrightnessState> {
        self.session.brightness(handle).ok()
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
    let mut deadline: Option<Instant> = None;
    loop {
        let timeout = runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .next_night_wait();
        let now = Instant::now();
        deadline = timeout.map(|wait| deadline.unwrap_or(now + wait).min(now + wait));
        let timeout = deadline.map(|until| until.saturating_duration_since(now));
        let Ok(Some(command)) = receive_commands(rx, timeout) else {
            break;
        };
        let mut runtime = runtime
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if deadline.is_some_and(|until| Instant::now() >= until) {
            if !matches!(command, Command::Tick) {
                runtime.handle(Command::Tick);
            }
            deadline = None;
        }
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
    let (tx, rx) = mpsc::channel();
    if !core_daemon::start_request_listener(&DAEMON_CONFIG, tx.clone(), parse_request) {
        return Err("failed to start plugin-monitor daemon listener".into());
    }
    set_live_state(Arc::clone(&runtime));
    let recovery = runtime.lock().unwrap().start(&device);
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

fn trace_layout_restore(report: &crate::session::RestoreReport) {
    #[cfg(debug_assertions)]
    qol_runtime::probe!(
        "MONITOR_SESSION",
        "event=layout_restore restored={} skipped={} failed={}",
        report.restored,
        report.skipped_display_gone,
        report.failed
    );
    #[cfg(not(debug_assertions))]
    let _ = report;
}

fn trace_gamma_reassert(report: &crate::session::RestoreReport) {
    #[cfg(debug_assertions)]
    if report.restored > 0 || report.failed > 0 {
        qol_runtime::probe!(
            "MONITOR_SESSION",
            "event=gamma_reassert restored={} failed={}",
            report.restored,
            report.failed
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = report;
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
    use crate::monitor::layout::LayoutPosition;
    use crate::monitor::{
        BrightnessPolicy, BrightnessSource, DisplayCapabilities, DisplayMode, GammaState,
        GammaTable, HdrState, RestoreOutcome,
    };
    use crate::session::NoLutProvider;
    use qol_windowing::MonitorBounds;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Condvar, Mutex as StdMutex};

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
        fail_neutral: AtomicUsize,
        snapshots: StdMutex<Vec<DisplaySnapshot>>,
        modes: StdMutex<BTreeMap<String, Vec<DisplayMode>>>,
        mode_calls: StdMutex<Vec<(String, DisplayMode)>>,
        mode_fail: AtomicBool,
        layout_calls: StdMutex<Vec<Vec<DisplayPlacement>>>,
        layout_fail: AtomicBool,
        gamma_calls: StdMutex<Vec<(String, u8, Tint)>>,
        snapshot_fail: AtomicBool,
        claim_checks: StdMutex<Vec<Option<Option<u32>>>>,
        layout_store: StdMutex<Option<SessionStore>>,
        events: StdMutex<Vec<String>>,
        snapshot_gate: StdMutex<Option<Arc<SnapshotGate>>>,
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
                fail_neutral: AtomicUsize::new(0),
                snapshots: StdMutex::new(Vec::new()),
                modes: StdMutex::new(BTreeMap::new()),
                mode_calls: StdMutex::new(Vec::new()),
                mode_fail: AtomicBool::new(false),
                layout_calls: StdMutex::new(Vec::new()),
                layout_fail: AtomicBool::new(false),
                gamma_calls: StdMutex::new(Vec::new()),
                snapshot_fail: AtomicBool::new(false),
                claim_checks: StdMutex::new(Vec::new()),
                layout_store: StdMutex::new(None),
                events: StdMutex::new(Vec::new()),
                snapshot_gate: StdMutex::new(None),
            }
        }

        fn calls(&self) -> Vec<(String, u8)> {
            self.calls.lock().unwrap().clone()
        }

        fn tints(&self) -> Vec<(String, Tint)> {
            self.tints.lock().unwrap().clone()
        }

        fn with_snapshot(self, snapshot: DisplaySnapshot) -> Self {
            self.snapshots.lock().unwrap().push(snapshot);
            self
        }

        fn with_modes(self, id: &str, modes: Vec<DisplayMode>) -> Self {
            self.modes.lock().unwrap().insert(id.to_string(), modes);
            self
        }

        fn with_store(self, store: SessionStore) -> Self {
            *self.layout_store.lock().unwrap() = Some(store);
            self
        }

        fn with_snapshot_gate(self, gate: Arc<SnapshotGate>) -> Self {
            *self.snapshot_gate.lock().unwrap() = Some(gate);
            self
        }

        fn observe_claim(&self) {
            let state = {
                let store = self.layout_store.lock().unwrap();
                store.as_ref().map(|store| {
                    store
                        .load_layout()
                        .ok()
                        .flatten()
                        .map(|snapshot| snapshot.mutations)
                })
            };
            self.claim_checks.lock().unwrap().push(state);
        }

        fn mode_calls(&self) -> Vec<(String, DisplayMode)> {
            self.mode_calls.lock().unwrap().clone()
        }

        fn layout_calls(&self) -> Vec<Vec<DisplayPlacement>> {
            self.layout_calls.lock().unwrap().clone()
        }

        fn gamma_calls(&self) -> Vec<(String, u8, Tint)> {
            self.gamma_calls.lock().unwrap().clone()
        }

        fn claim_checks(&self) -> Vec<Option<Option<u32>>> {
            self.claim_checks.lock().unwrap().clone()
        }

        fn events(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }
    }

    struct SnapshotGate {
        entered: (StdMutex<bool>, Condvar),
        release: (StdMutex<bool>, Condvar),
        armed: AtomicBool,
    }

    impl SnapshotGate {
        fn new() -> Self {
            Self {
                entered: (StdMutex::new(false), Condvar::new()),
                release: (StdMutex::new(false), Condvar::new()),
                armed: AtomicBool::new(false),
            }
        }

        fn block(&self) {
            if self.armed.swap(true, Ordering::SeqCst) {
                return;
            }
            *self.entered.0.lock().unwrap() = true;
            self.entered.1.notify_all();
            let mut released = self.release.0.lock().unwrap();
            while !*released {
                released = self.release.1.wait(released).unwrap();
            }
        }

        fn wait_entered(&self) -> bool {
            let mut entered = self.entered.0.lock().unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            while !*entered && Instant::now() < deadline {
                let (guard, _) = self
                    .entered
                    .1
                    .wait_timeout(entered, Duration::from_millis(25))
                    .unwrap();
                entered = guard;
            }
            *entered
        }

        fn release(&self) {
            *self.release.0.lock().unwrap() = true;
            self.release.1.notify_all();
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
            if tint.is_neutral()
                && self
                    .fail_neutral
                    .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                        count.checked_sub(1)
                    })
                    .is_ok()
            {
                return Err(MonitorError::refused("tint", "temporary display failure"));
            }
            self.tints
                .lock()
                .unwrap()
                .push((handle.id().to_string(), tint));
            Ok(())
        }

        fn set_brightness_with_tint(
            &self,
            handle: &DisplayHandle,
            value: u8,
            tint: Tint,
        ) -> Result<(), MonitorError> {
            if self.source == BrightnessSource::Gamma {
                self.set_gamma_adjustment(handle, value, tint)
            } else {
                self.set_brightness(handle, value)
            }
        }

        fn set_gamma_adjustment(
            &self,
            handle: &DisplayHandle,
            value: u8,
            tint: Tint,
        ) -> Result<(), MonitorError> {
            self.gamma_calls
                .lock()
                .unwrap()
                .push((handle.id().to_string(), value, tint));
            self.events
                .lock()
                .unwrap()
                .push(format!("gamma:{}", handle.id()));
            self.set_tint(handle, tint)?;
            if self.source == BrightnessSource::Gamma {
                self.set_brightness(handle, value)?;
            }
            Ok(())
        }

        fn get_gamma(&self, _handle: &DisplayHandle) -> Result<GammaState, MonitorError> {
            Err(MonitorError::unsupported("gamma", "test"))
        }

        fn set_gamma(&self, _handle: &DisplayHandle, _value: u8) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("gamma", "test"))
        }

        fn list_modes(&self, handle: &DisplayHandle) -> Result<Vec<DisplayMode>, MonitorError> {
            self.modes
                .lock()
                .unwrap()
                .get(handle.id())
                .cloned()
                .ok_or_else(|| MonitorError::unsupported("modes", "test"))
        }

        fn set_mode(&self, handle: &DisplayHandle, mode: &DisplayMode) -> Result<(), MonitorError> {
            self.observe_claim();
            if self.mode_fail.load(Ordering::SeqCst) {
                return Err(MonitorError::refused("mode", "injected"));
            }
            self.mode_calls
                .lock()
                .unwrap()
                .push((handle.id().to_string(), mode.clone()));
            self.events
                .lock()
                .unwrap()
                .push(format!("set_mode:{}", handle.id()));
            if let Some(snapshot) = self
                .snapshots
                .lock()
                .unwrap()
                .iter_mut()
                .find(|snapshot| snapshot.handle.id() == handle.id())
            {
                snapshot.mode = Some(mode.clone());
            }
            Ok(())
        }

        fn get_hdr(&self, _handle: &DisplayHandle) -> Result<HdrState, MonitorError> {
            Err(MonitorError::unsupported("hdr", "test"))
        }

        fn set_hdr(&self, _handle: &DisplayHandle, _enabled: bool) -> Result<(), MonitorError> {
            Err(MonitorError::unsupported("hdr", "test"))
        }

        fn snapshot(&self) -> Result<Vec<DisplaySnapshot>, MonitorError> {
            if let Some(gate) = self.snapshot_gate.lock().unwrap().clone() {
                gate.block();
            }
            if self.snapshot_fail.load(Ordering::SeqCst) {
                return Err(MonitorError::refused("layout", "snapshot unavailable"));
            }
            Ok(self.snapshots.lock().unwrap().clone())
        }

        fn set_layout(&self, placements: &[DisplayPlacement]) -> Result<(), MonitorError> {
            self.observe_claim();
            if self.layout_fail.load(Ordering::SeqCst) {
                return Err(MonitorError::refused("layout", "injected"));
            }
            {
                let mut snapshots = self.snapshots.lock().unwrap();
                for snapshot in snapshots.iter_mut() {
                    if let Some(placement) = placements
                        .iter()
                        .find(|placement| placement.handle.id() == snapshot.handle.id())
                    {
                        snapshot.bounds.x = placement.x as f32;
                        snapshot.bounds.y = placement.y as f32;
                        snapshot.primary = placement.primary;
                    }
                }
            }
            self.layout_calls.lock().unwrap().push(placements.to_vec());
            self.events
                .lock()
                .unwrap()
                .push(format!("set_layout:{}", placements.len()));
            Ok(())
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

    struct StaticLut(Option<GammaTable>);

    impl LutProvider for StaticLut {
        fn capture(&self, _connector: &str) -> Option<GammaTable> {
            Some(self.0.clone().unwrap_or_else(|| GammaTable {
                red: vec![0, u16::MAX],
                green: vec![0, u16::MAX],
                blue: vec![0, u16::MAX],
            }))
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
            Arc::new(StaticLut(None)),
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
    fn failed_schedule_off_is_retried_until_the_display_is_neutral() {
        let (_session_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        let control = Arc::new(FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            70,
            BrightnessSource::Ddc,
        ));
        let clock = Arc::new(StdMutex::new(Now {
            unix: 75_600,
            minute: Minute(21 * 60),
        }));
        let mut runtime = night_runtime(control.clone(), store, config_root, clock.clone());
        runtime.start(&DeviceConfig {
            night_schedule: "daily".into(),
            ..DeviceConfig::default()
        });
        control.fail_neutral.store(1, Ordering::SeqCst);
        *clock.lock().unwrap() = Now {
            unix: 108_000,
            minute: Minute(6 * 60),
        };
        runtime.handle(Command::Tick);
        assert_eq!(runtime.night_payload()["state"], "failed");
        assert!(runtime.session.tinted_displays().contains("id-1"));
        runtime.handle(Command::Tick);
        assert_eq!(control.tints().last().unwrap().1, Tint::NEUTRAL);
        assert_eq!(runtime.night_payload()["state"], "inactive");
        assert!(runtime.session.tinted_displays().is_empty());
    }

    #[test]
    fn restarting_after_the_schedule_end_clears_an_inherited_tint() {
        let (_session_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        let control = Arc::new(FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            70,
            BrightnessSource::Ddc,
        ));
        let mut snapshot = stale_snapshot("id-1", "card0-DP-1", 70, 70);
        snapshot.handoff = true;
        snapshot.adopt_generation = Some("next".into());
        snapshot.last_tint = Tint::from_kelvin(3500);
        store.write_snapshot(&snapshot).unwrap();
        let clock = Arc::new(StdMutex::new(Now {
            unix: 108_000,
            minute: Minute(6 * 60),
        }));
        let mut runtime = night_runtime(control.clone(), store, config_root, clock)
            .with_adoption_generation(Some("next".into()));
        runtime.start(&DeviceConfig {
            night_schedule: "daily".into(),
            ..DeviceConfig::default()
        });
        assert_eq!(control.tints().last().unwrap().1, Tint::NEUTRAL);
        assert_eq!(runtime.night_payload()["state"], "inactive");
    }

    #[derive(Default)]
    struct NativeNightLight {
        calls: StdMutex<Vec<(bool, u16)>>,
        owned: std::sync::atomic::AtomicBool,
        fail: std::sync::atomic::AtomicBool,
        fail_release: std::sync::atomic::AtomicBool,
    }

    impl HostNightLight for NativeNightLight {
        fn native_supported(&self) -> bool {
            true
        }
        fn strategy(&self) -> &'static str {
            "native-test"
        }
        fn apply_native(&self, active: bool, kelvin: u16) -> Result<bool, HostNightLightError> {
            if self.fail.swap(false, Ordering::SeqCst) {
                return Err(HostNightLightError::Failed(
                    "temporary native failure".into(),
                ));
            }
            self.calls.lock().unwrap().push((active, kelvin));
            self.owned.store(true, Ordering::SeqCst);
            Ok(true)
        }
        fn take_over(&self) -> Result<TakeoverOutcome, HostNightLightError> {
            self.owned.store(true, Ordering::SeqCst);
            Ok(TakeoverOutcome::AlreadyOff)
        }
        fn release(&self, _mode: RestoreMode) -> Result<(), HostNightLightError> {
            if self.fail_release.swap(false, Ordering::SeqCst) {
                return Err(HostNightLightError::Failed(
                    "temporary release failure".into(),
                ));
            }
            self.owned.store(false, Ordering::SeqCst);
            Ok(())
        }
        fn mark_handoff(&self, _successor: Option<&str>) {}
        fn is_taken_over(&self) -> bool {
            self.owned.load(Ordering::SeqCst)
        }
        fn status(&self) -> HostNightLightStatus {
            HostNightLightStatus::TakenOver
        }
    }

    #[test]
    fn resident_restart_clears_persisted_tint_from_brightness_adoption() {
        for native in [false, true] {
            for handoff in [false, true] {
                for schedule in ["off", "daily"] {
                    let (_session_dir, store) = runtime_store();
                    let (_root, config_root) = preferred_root();
                    let control = Arc::new(FakeControl::new(
                        vec![handle("id-1", "card0-DP-1")],
                        70,
                        BrightnessSource::Gamma,
                    ));
                    let baseline = StaticLut(None).capture("card0-DP-1").unwrap();
                    let warm = Tint::from_kelvin(3500);
                    let current = baseline.dimmed(70).tinted(warm);
                    store
                        .write_snapshot(&Snapshot {
                            source: "gamma".into(),
                            lut: Some(baseline),
                            last_tint: warm,
                            handoff,
                            adopt_generation: Some("old-generation".into()),
                            ..stale_snapshot("id-1", "card0-DP-1", 100, 70)
                        })
                        .unwrap();
                    if handoff {
                        write_preferred(&config_root, BTreeMap::from([("id-1".into(), 80)]));
                    }
                    let mut runtime = Runtime::new(
                        control.clone(),
                        store.clone(),
                        Arc::new(StaticLut(Some(current))),
                        |_title, _body| {},
                        Some(config_root),
                        |_preferred| Ok(()),
                        || true,
                    )
                    .with_adoption_generation(Some("new-generation".into()))
                    .with_clock(|| Now {
                        unix: 108_000,
                        minute: Minute(6 * 60),
                    });
                    if native {
                        runtime =
                            runtime.with_host_night_light(Arc::new(NativeNightLight::default()));
                    }
                    runtime.start(&DeviceConfig {
                        night_schedule: schedule.into(),
                        ..DeviceConfig::default()
                    });
                    assert_eq!(control.tints().last().unwrap().1, Tint::NEUTRAL);
                    assert!(runtime.session.tinted_displays().is_empty());
                    assert_eq!(runtime.night_payload()["state"], "inactive");
                    let snapshot = store.load_snapshot("id-1").unwrap().unwrap();
                    assert_eq!(snapshot.last_value, if handoff { 80 } else { 70 });
                    assert!(snapshot.last_tint.is_neutral());
                }
            }
        }
    }

    #[test]
    fn brightness_steps_and_night_toggles_share_the_session_adjustment() {
        for source in [BrightnessSource::Gamma, BrightnessSource::Ddc] {
            let (_dir, store) = runtime_store();
            let (_root, config_root) = preferred_root();
            let display = handle("id-1", "card0-DP-1");
            let control = Arc::new(FakeControl::new(vec![display.clone()], 75, source));
            let native = Arc::new(NativeNightLight::default());
            let clock = Arc::new(StdMutex::new(Now {
                unix: 100_000,
                minute: Minute(12 * 60),
            }));
            let mut runtime = night_runtime(control.clone(), store, config_root, clock)
                .with_host_night_light(native.clone());
            runtime.start(&DeviceConfig::default());
            runtime.step(-1);
            runtime.handle(Command::Night(NightRequest::On));
            assert_eq!(runtime.session.brightness(&display).unwrap().value, 70);
            runtime.step(1);
            runtime.handle(Command::Night(NightRequest::Off));
            assert_eq!(runtime.session.brightness(&display).unwrap().value, 75);
            assert_eq!(control.current.lock().unwrap()["id-1"], 75);
            if source == BrightnessSource::Gamma {
                assert!(native.calls.lock().unwrap().is_empty());
                assert_eq!(
                    control.tints(),
                    vec![
                        ("id-1".into(), Tint::NEUTRAL),
                        ("id-1".into(), Tint::from_kelvin(3500)),
                        ("id-1".into(), Tint::from_kelvin(3500)),
                        ("id-1".into(), Tint::NEUTRAL)
                    ]
                );
                assert!(runtime.night_payload()["fallback_reason"]
                    .as_str()
                    .unwrap()
                    .contains("brightness"));
            } else {
                assert_eq!(*native.calls.lock().unwrap(), [(true, 3500), (false, 3500)]);
                assert!(control.tints().is_empty());
            }
        }
    }

    #[test]
    fn native_strategy_drives_the_schedule_without_gamma_and_retries_failures() {
        let (_session_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        let control = Arc::new(FakeControl::new(vec![], 70, BrightnessSource::Ddc));
        let native = Arc::new(NativeNightLight::default());
        let clock = Arc::new(StdMutex::new(Now {
            unix: 75_600,
            minute: Minute(21 * 60),
        }));
        let mut runtime = night_runtime(control.clone(), store, config_root, clock.clone())
            .with_host_night_light(native.clone());
        runtime.start(&DeviceConfig {
            night_schedule: "daily".into(),
            ..DeviceConfig::default()
        });
        assert_eq!(*native.calls.lock().unwrap(), [(true, 3500)]);
        runtime.handle(Command::Tick);
        assert_eq!(native.calls.lock().unwrap().len(), 1);
        *clock.lock().unwrap() = Now {
            unix: 108_000,
            minute: Minute(6 * 60),
        };
        native.fail.store(true, Ordering::SeqCst);
        runtime.handle(Command::Tick);
        assert_eq!(runtime.night_payload()["state"], "failed");
        runtime.handle(Command::Tick);
        assert_eq!(native.calls.lock().unwrap().last(), Some(&(false, 3500)));
        assert_eq!(runtime.night_payload()["state"], "inactive");
        assert_eq!(runtime.night_payload()["strategy"], "native-test");
        assert!(control.tints().is_empty());
    }

    #[test]
    fn failed_native_release_keeps_a_retry_armed_in_manual_mode() {
        let (_session_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        let control = Arc::new(FakeControl::new(vec![], 70, BrightnessSource::Ddc));
        let native = Arc::new(NativeNightLight::default());
        let clock = Arc::new(StdMutex::new(Now {
            unix: 75_600,
            minute: Minute(21 * 60),
        }));
        let mut runtime =
            night_runtime(control, store, config_root, clock).with_host_night_light(native.clone());
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::Night(NightRequest::On));
        native.fail_release.store(true, Ordering::SeqCst);
        runtime.handle(Command::Night(NightRequest::Off));
        assert_eq!(runtime.night_payload()["state"], "failed");
        assert!(runtime.next_night_wait().is_some());
        runtime.handle(Command::Tick);
        assert_eq!(runtime.night_payload()["state"], "inactive");
        assert!(!native.is_taken_over());
        assert_eq!(runtime.next_night_wait(), None);
    }

    #[test]
    fn unavailable_native_control_falls_back_to_display_gamma_with_a_reason() {
        let (_session_dir, store) = runtime_store();
        let (_root, config_root) = preferred_root();
        let control = Arc::new(FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            70,
            BrightnessSource::Ddc,
        ));
        let clock = Arc::new(StdMutex::new(Now {
            unix: 75_600,
            minute: Minute(21 * 60),
        }));
        let mut runtime = night_runtime(control.clone(), store, config_root, clock)
            .with_host_night_light(Arc::new(
                crate::host_night_light::UnavailableHostNightLight("native unavailable"),
            ));
        runtime.start(&DeviceConfig {
            night_schedule: "daily".into(),
            ..DeviceConfig::default()
        });
        assert_eq!(runtime.night_payload()["state"], "active");
        assert_eq!(runtime.night_payload()["strategy"], "gamma");
        assert_eq!(
            runtime.night_payload()["fallback_reason"],
            "native unavailable"
        );
        assert_eq!(control.tints().last().unwrap().1, Tint::from_kelvin(3500));
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
            &mut runtime.session.brightness_states(),
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
        assert_eq!(runtime.session.brightness_states()["id-1"].value, 70);
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

    fn display_mode(token: u64, width: u32, height: u32, refresh_hz: u32) -> DisplayMode {
        DisplayMode {
            token,
            width,
            height,
            refresh_hz,
        }
    }

    fn display_snapshot(
        id: &str,
        connector: &str,
        x: f32,
        y: f32,
        primary: bool,
        mode: Option<DisplayMode>,
    ) -> DisplaySnapshot {
        DisplaySnapshot {
            handle: handle(id, connector),
            bounds: MonitorBounds {
                x,
                y,
                width: 1920.0,
                height: 1080.0,
            },
            primary,
            mode,
        }
    }

    fn layout_runtime(
        control: Arc<FakeControl>,
        store: SessionStore,
        resident: bool,
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
            move || resident,
        );
        (runtime, toasts)
    }

    fn layout_capture(placements: Vec<PlacementRecord>, modes: Vec<ModeRecord>) -> LayoutSnapshot {
        LayoutSnapshot {
            schema_version: crate::session::LAYOUT_SCHEMA_VERSION,
            layout_id: crate::session::LAYOUT_SNAPSHOT_ID.to_string(),
            placements,
            modes,
            mutations: 0,
            handoff: false,
            adopt_generation: None,
        }
    }

    fn gamma_lut() -> GammaTable {
        GammaTable {
            red: vec![1000, 2000],
            green: vec![1000, 2000],
            blue: vec![1000, 2000],
        }
    }

    #[test]
    fn parses_set_mode_with_and_without_a_token_and_refresh() {
        assert!(matches!(
            parse_request(&request(
                "set_mode",
                serde_json::json!({ "id": "id-1", "width": 1920, "height": 1080 })
            )),
            ReadResult::Command(Command::SetMode { display, token, width, height, refresh })
                if display == "id-1"
                    && token.is_none()
                    && width == 1920
                    && height == 1080
                    && refresh.is_none()
        ));
        assert!(matches!(
            parse_request(&request(
                "set_mode",
                serde_json::json!({ "id": "id-1", "token": 11, "width": 1280, "height": 720, "refresh": 75 })
            )),
            ReadResult::Command(Command::SetMode {
                token: Some(11),
                refresh: Some(75),
                ..
            })
        ));
    }

    #[test]
    fn rejects_set_mode_with_missing_wrong_or_out_of_range_input() {
        for input in [
            serde_json::json!({ "width": 1920, "height": 1080 }),
            serde_json::json!({ "id": "", "width": 1920, "height": 1080 }),
            serde_json::json!({ "id": "id-1", "height": 1080 }),
            serde_json::json!({ "id": "id-1", "width": "1920", "height": 1080 }),
            serde_json::json!({ "id": "id-1", "width": 1920.5, "height": 1080 }),
            serde_json::json!({ "id": "id-1", "width": 0, "height": 1080 }),
            serde_json::json!({ "id": "id-1", "width": 4_294_967_296u64, "height": 1080 }),
            serde_json::json!({ "id": "id-1", "width": 1920, "height": 1080, "refresh": 0 }),
            serde_json::json!({ "id": "id-1", "width": 1920, "height": 1080, "refresh": "60" }),
            serde_json::json!({ "id": "id-1", "token": 11, "height": 1080 }),
            serde_json::json!({ "id": "id-1", "token": 11, "width": 1920 }),
            serde_json::json!({ "id": "id-1", "token": "11", "width": 1920, "height": 1080 }),
            serde_json::json!({ "id": "id-1", "token": -1, "width": 1920, "height": 1080 }),
            serde_json::json!({ "id": "id-1", "token": 1.5, "width": 1920, "height": 1080 }),
        ] {
            assert!(
                matches!(
                    parse_request(&request("set_mode", input.clone())),
                    ReadResult::Error(_)
                ),
                "input: {input}"
            );
        }
    }

    #[test]
    fn parses_set_primary_and_rejects_a_missing_id() {
        assert!(matches!(
            parse_request(&request("set_primary", serde_json::json!({ "id": "id-1" }))),
            ReadResult::Command(Command::SetPrimary { display }) if display == "id-1"
        ));
        for input in [
            serde_json::Value::Null,
            serde_json::json!({ "id": "" }),
            serde_json::json!({ "id": 4 }),
        ] {
            assert!(matches!(
                parse_request(&request("set_primary", input.clone())),
                ReadResult::Error(_)
            ));
        }
    }

    #[test]
    fn parses_arrange_with_and_without_a_primary() {
        assert!(matches!(
            parse_request(&request(
                "arrange",
                serde_json::json!({ "placements": [{ "id": "id-1", "x": 0, "y": 0 }] })
            )),
            ReadResult::Command(Command::Arrange { placements, primary: None })
                if placements.len() == 1 && placements[0].id == "id-1"
        ));
        assert!(matches!(
            parse_request(&request(
                "arrange",
                serde_json::json!({
                    "placements": [{ "id": "id-1", "x": -1920, "y": 0 }],
                    "primary": "id-1"
                })
            )),
            ReadResult::Command(Command::Arrange { primary: Some(id), .. }) if id == "id-1"
        ));
    }

    #[test]
    fn rejects_arrange_with_missing_wrong_or_out_of_range_input() {
        for input in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!({ "placements": [] }),
            serde_json::json!({ "placements": [{ "id": "id-1", "x": 0 }] }),
            serde_json::json!({ "placements": [{ "id": "id-1", "x": "0", "y": 0 }] }),
            serde_json::json!({ "placements": [{ "id": "id-1", "x": 0, "y": 4_294_967_296u64 }] }),
            serde_json::json!({ "placements": [{ "id": "id-1", "x": 0, "y": 0 }], "primary": "" }),
        ] {
            assert!(
                matches!(
                    parse_request(&request("arrange", input.clone())),
                    ReadResult::Error(_)
                ),
                "input: {input}"
            );
        }
    }

    #[test]
    fn routes_apply_layout_as_a_command() {
        assert!(matches!(
            parse_request(&request("apply_layout", serde_json::Value::Null)),
            ReadResult::Command(Command::ApplyLayout)
        ));
    }

    #[test]
    fn layout_query_reports_rows_and_unavailable_state() {
        let control = FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            70,
            BrightnessSource::Ddc,
        )
        .with_snapshot(display_snapshot(
            "id-1",
            "card0-DP-1",
            100.0,
            50.0,
            true,
            Some(display_mode(1, 1920, 1080, 60)),
        ));
        let payload = layout_payload(&control);
        assert!(payload.is_array(), "layout payload must be a bare array");
        assert_eq!(payload[0]["connector"], "card0-DP-1");
        assert_eq!(payload[0]["x"], serde_json::json!(100));
        assert_eq!(payload[0]["y"], serde_json::json!(50));
        assert_eq!(payload[0]["primary"], serde_json::json!(true));
        assert!(payload[0]["detail"].is_string());
        control.snapshot_fail.store(true, Ordering::SeqCst);
        let unavailable = layout_payload(&control);
        assert_eq!(unavailable, serde_json::json!([]));
    }

    #[test]
    fn modes_query_skips_a_display_whose_mode_list_fails() {
        let control = FakeControl::new(
            vec![handle("id-1", "card0-DP-1"), handle("id-2", "card0-HDMI-1")],
            70,
            BrightnessSource::Ddc,
        )
        .with_snapshot(display_snapshot(
            "id-1",
            "card0-DP-1",
            0.0,
            0.0,
            true,
            Some(display_mode(1, 1920, 1080, 60)),
        ))
        .with_snapshot(display_snapshot(
            "id-2",
            "card0-HDMI-1",
            1920.0,
            0.0,
            false,
            None,
        ))
        .with_modes(
            "id-1",
            vec![
                display_mode(1, 1920, 1080, 60),
                display_mode(2, 1280, 720, 60),
            ],
        );
        let payload = modes_payload(&control);
        assert!(payload.is_array(), "modes payload must be a bare array");
        let rows = payload.as_array().unwrap();
        assert_eq!(
            rows.len(),
            2,
            "only the display with a readable mode list contributes rows"
        );
        assert!(rows.iter().all(|row| row["connector"] == "card0-DP-1"));
        let current = rows
            .iter()
            .filter(|row| row["current"] == serde_json::json!(true))
            .count();
        assert_eq!(current, 1, "the current mode is marked exactly once");
    }

    #[test]
    fn displays_payload_appends_geometry_mode_and_primary() {
        let control = FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            70,
            BrightnessSource::Ddc,
        )
        .with_snapshot(display_snapshot(
            "id-1",
            "card0-DP-1",
            100.0,
            50.0,
            true,
            Some(display_mode(1, 1920, 1080, 60)),
        ));
        let rows = displays_payload(&control, &BTreeMap::new(), &mut BTreeMap::new(), None);
        let detail = rows[0]["detail"].as_str().unwrap();
        assert!(detail.contains("+100+50"), "detail: {detail}");
        assert!(detail.contains("1920x1080@60Hz"), "detail: {detail}");
        assert!(detail.contains("primary"), "detail: {detail}");
    }

    #[test]
    fn set_mode_records_the_mutation_before_the_first_write() {
        let (_dir, store) = runtime_store();
        store
            .write_snapshot(&crate::session::Snapshot {
                source: "gamma".into(),
                lut: Some(gamma_lut()),
                last_value: 40,
                ..stale_snapshot("id-1", "card0-DP-1", 100, 40)
            })
            .unwrap();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot(
                "id-1",
                "card0-DP-1",
                0.0,
                0.0,
                true,
                Some(display_mode(1, 1920, 1080, 60)),
            ))
            .with_modes(
                "id-1",
                vec![
                    display_mode(1, 1920, 1080, 60),
                    display_mode(2, 1280, 720, 60),
                ],
            )
            .with_store(store.clone()),
        );
        let (mut runtime, _toasts) = layout_runtime(control.clone(), store.clone(), false);
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::SetMode {
            display: "id-1".into(),
            token: None,
            width: 1280,
            height: 720,
            refresh: Some(60),
        });
        assert_eq!(
            control.claim_checks(),
            vec![Some(Some(1))],
            "the mutation is recorded before the mode write"
        );
        assert_eq!(
            control.mode_calls(),
            vec![("id-1".to_string(), display_mode(2, 1280, 720, 60))]
        );
        assert_eq!(
            control.events(),
            vec!["set_mode:id-1".to_string(), "gamma:id-1".to_string()],
            "the gamma re-assert runs after the mode write"
        );
        assert_eq!(
            control.gamma_calls(),
            vec![("id-1".to_string(), 40, Tint::NEUTRAL)]
        );
        let snapshot = store.load_layout().unwrap().unwrap();
        assert_eq!(snapshot.mutations, 1);
        assert_eq!(snapshot.placements[0].id, "id-1");
        assert_eq!(snapshot.modes[0].token, 1);
    }

    #[test]
    fn mode_write_reasserts_gamma_on_every_connected_display() {
        let (_dir, store) = runtime_store();
        store
            .write_snapshot(&crate::session::Snapshot {
                source: "gamma".into(),
                lut: Some(gamma_lut()),
                last_value: 40,
                ..stale_snapshot("id-1", "card0-DP-1", 100, 40)
            })
            .unwrap();
        store
            .write_snapshot(&crate::session::Snapshot {
                source: "gamma".into(),
                lut: Some(gamma_lut()),
                last_value: 55,
                ..stale_snapshot("id-2", "card0-HDMI-1", 100, 55)
            })
            .unwrap();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1"), handle("id-2", "card0-HDMI-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot(
                "id-1",
                "card0-DP-1",
                0.0,
                0.0,
                true,
                Some(display_mode(1, 1920, 1080, 60)),
            ))
            .with_snapshot(display_snapshot(
                "id-2",
                "card0-HDMI-1",
                1920.0,
                0.0,
                false,
                None,
            ))
            .with_modes(
                "id-1",
                vec![
                    display_mode(1, 1920, 1080, 60),
                    display_mode(2, 1280, 720, 60),
                ],
            )
            .with_store(store.clone()),
        );
        let (mut runtime, _toasts) = layout_runtime(control.clone(), store.clone(), false);
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::SetMode {
            display: "id-1".into(),
            token: None,
            width: 1280,
            height: 720,
            refresh: Some(60),
        });
        assert_eq!(
            control.mode_calls(),
            vec![("id-1".to_string(), display_mode(2, 1280, 720, 60))]
        );
        assert_eq!(
            control.gamma_calls(),
            vec![
                ("id-1".to_string(), 40, Tint::NEUTRAL),
                ("id-2".to_string(), 55, Tint::NEUTRAL),
            ],
            "the mode write re-asserts every connected display"
        );
    }

    #[test]
    fn night_tint_survives_a_mode_write() {
        let (_dir, store) = runtime_store();
        store
            .write_snapshot(&crate::session::Snapshot {
                source: "gamma".into(),
                lut: Some(gamma_lut()),
                last_value: 40,
                ..stale_snapshot("id-1", "card0-DP-1", 100, 40)
            })
            .unwrap();
        let (_root, config_root) = preferred_root();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot(
                "id-1",
                "card0-DP-1",
                0.0,
                0.0,
                true,
                Some(display_mode(1, 1920, 1080, 60)),
            ))
            .with_modes(
                "id-1",
                vec![
                    display_mode(1, 1920, 1080, 60),
                    display_mode(2, 1280, 720, 60),
                ],
            )
            .with_store(store.clone()),
        );
        let clock = Arc::new(StdMutex::new(Now {
            unix: 100_000,
            minute: Minute(12 * 60),
        }));
        let mut runtime = runtime_with_root(control.clone(), store.clone(), Some(config_root))
            .with_clock(move || *clock.lock().unwrap());
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::Night(NightRequest::Toggle));
        let warm = Tint::from_kelvin(3500);
        assert!(control
            .tints()
            .iter()
            .any(|(id, tint)| id == "id-1" && *tint == warm));
        runtime.handle(Command::SetMode {
            display: "id-1".into(),
            token: None,
            width: 1280,
            height: 720,
            refresh: Some(60),
        });
        let calls = control.gamma_calls();
        let last = calls
            .last()
            .expect("the mode write must reapply the display state");
        assert_eq!(last.0, "id-1");
        assert_eq!(last.1, 40, "the reassert keeps the recorded ramp");
        assert_eq!(last.2, warm, "the reassert keeps the night tint");
        assert_eq!(runtime.night_payload()["state"], "active");
    }

    #[test]
    fn refused_mode_write_triggers_no_reapply() {
        let (_dir, store) = runtime_store();
        store
            .write_snapshot(&crate::session::Snapshot {
                source: "gamma".into(),
                lut: Some(gamma_lut()),
                last_value: 40,
                ..stale_snapshot("id-1", "card0-DP-1", 100, 40)
            })
            .unwrap();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot(
                "id-1",
                "card0-DP-1",
                0.0,
                0.0,
                true,
                Some(display_mode(1, 1920, 1080, 60)),
            ))
            .with_modes(
                "id-1",
                vec![
                    display_mode(1, 1920, 1080, 60),
                    display_mode(2, 1280, 720, 60),
                ],
            )
            .with_store(store.clone()),
        );
        control.mode_fail.store(true, Ordering::SeqCst);
        let (mut runtime, toasts) = layout_runtime(control.clone(), store.clone(), false);
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::SetMode {
            display: "id-1".into(),
            token: None,
            width: 1280,
            height: 720,
            refresh: Some(60),
        });
        assert!(control.mode_calls().is_empty());
        assert!(
            control.gamma_calls().is_empty(),
            "a refused mode write must not re-assert any display"
        );
        assert!(toasts
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("Mode not set"));
    }

    #[test]
    fn set_mode_refuses_an_ambiguous_resolution_without_claiming_or_writing() {
        let (_dir, store) = runtime_store();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot(
                "id-1",
                "card0-DP-1",
                0.0,
                0.0,
                true,
                Some(display_mode(1, 1920, 1080, 60)),
            ))
            .with_modes(
                "id-1",
                vec![
                    display_mode(1, 1920, 1080, 60),
                    display_mode(2, 1920, 1080, 50),
                ],
            )
            .with_store(store.clone()),
        );
        let (mut runtime, toasts) = layout_runtime(control.clone(), store.clone(), false);
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::SetMode {
            display: "id-1".into(),
            token: None,
            width: 1920,
            height: 1080,
            refresh: None,
        });
        assert!(store.load_layout().unwrap().is_none());
        assert!(control.claim_checks().is_empty());
        assert!(control.mode_calls().is_empty());
        assert!(toasts
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("ambiguous without a refresh rate"));
    }

    #[test]
    fn set_mode_reports_an_unknown_display_without_claiming() {
        let (_dir, store) = runtime_store();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_store(store.clone()),
        );
        let (mut runtime, toasts) = layout_runtime(control.clone(), store.clone(), false);
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::SetMode {
            display: "id-gone".into(),
            token: None,
            width: 1280,
            height: 720,
            refresh: None,
        });
        assert!(store.load_layout().unwrap().is_none());
        assert!(control.claim_checks().is_empty());
        assert!(toasts.lock().unwrap().last().unwrap().contains("id-gone"));
    }

    #[test]
    fn set_mode_prefers_the_token_over_a_colliding_label() {
        let (_dir, store) = runtime_store();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot(
                "id-1",
                "card0-DP-1",
                0.0,
                0.0,
                true,
                Some(display_mode(10, 1920, 1080, 60)),
            ))
            .with_modes(
                "id-1",
                vec![
                    display_mode(10, 1920, 1080, 60),
                    display_mode(11, 1920, 1080, 60),
                ],
            )
            .with_store(store.clone()),
        );
        let (mut runtime, _toasts) = layout_runtime(control.clone(), store.clone(), false);
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::SetMode {
            display: "id-1".into(),
            token: Some(11),
            width: 1920,
            height: 1080,
            refresh: Some(60),
        });
        assert_eq!(
            control.mode_calls(),
            vec![("id-1".to_string(), display_mode(11, 1920, 1080, 60))],
            "the second colliding row must address its own token"
        );
        assert_eq!(store.load_layout().unwrap().unwrap().mutations, 1);
    }

    #[test]
    fn set_mode_refuses_an_unknown_token_without_claiming() {
        let (_dir, store) = runtime_store();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot(
                "id-1",
                "card0-DP-1",
                0.0,
                0.0,
                true,
                Some(display_mode(10, 1920, 1080, 60)),
            ))
            .with_modes("id-1", vec![display_mode(10, 1920, 1080, 60)])
            .with_store(store.clone()),
        );
        let (mut runtime, toasts) = layout_runtime(control.clone(), store.clone(), false);
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::SetMode {
            display: "id-1".into(),
            token: Some(99),
            width: 1920,
            height: 1080,
            refresh: Some(60),
        });
        assert!(store.load_layout().unwrap().is_none());
        assert!(control.claim_checks().is_empty());
        assert!(control.mode_calls().is_empty());
        assert!(toasts.lock().unwrap().last().unwrap().contains("token 99"));
    }

    #[test]
    fn set_primary_applies_one_primary_layout_and_reasserts_gamma() {
        let (_dir, store) = runtime_store();
        store
            .write_snapshot(&crate::session::Snapshot {
                source: "gamma".into(),
                lut: Some(gamma_lut()),
                last_value: 60,
                ..stale_snapshot("id-2", "card0-HDMI-1", 100, 60)
            })
            .unwrap();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1"), handle("id-2", "card0-HDMI-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot("id-1", "card0-DP-1", 0.0, 0.0, true, None))
            .with_snapshot(display_snapshot(
                "id-2",
                "card0-HDMI-1",
                1920.0,
                0.0,
                false,
                None,
            ))
            .with_store(store.clone()),
        );
        let (mut runtime, _toasts) = layout_runtime(control.clone(), store.clone(), false);
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::SetPrimary {
            display: "id-2".into(),
        });
        let calls = control.layout_calls();
        assert_eq!(calls.len(), 1);
        assert!(!calls[0][0].primary);
        assert!(calls[0][1].primary);
        assert_eq!(control.claim_checks(), vec![Some(Some(1))]);
        let snapshot = store.load_layout().unwrap().unwrap();
        assert_eq!(snapshot.mutations, 1);
        assert!(snapshot
            .placements
            .iter()
            .any(|record| record.id == "id-1" && record.primary));
        assert_eq!(
            control.events(),
            vec!["set_layout:2".to_string(), "gamma:id-2".to_string()],
            "the gamma re-assert runs after the layout write"
        );
        assert_eq!(
            control.gamma_calls(),
            vec![("id-2".to_string(), 60, Tint::NEUTRAL)]
        );
    }

    #[test]
    fn arrange_unknown_display_refuses_without_claiming() {
        let (_dir, store) = runtime_store();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot("id-1", "card0-DP-1", 0.0, 0.0, true, None))
            .with_store(store.clone()),
        );
        let (mut runtime, toasts) = layout_runtime(control.clone(), store.clone(), false);
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::Arrange {
            placements: vec![ArrangeRequest {
                id: "id-gone".into(),
                x: 0,
                y: 0,
            }],
            primary: None,
        });
        assert!(store.load_layout().unwrap().is_none());
        assert!(control.claim_checks().is_empty());
        assert!(control.layout_calls().is_empty());
        assert!(toasts.lock().unwrap().last().unwrap().contains("id-gone"));
    }

    #[test]
    fn apply_layout_uses_the_configured_positions() {
        let (_dir, store) = runtime_store();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot("id-1", "card0-DP-1", 0.0, 0.0, true, None))
            .with_store(store.clone()),
        );
        let config = DeviceConfig {
            layout_position: BTreeMap::from([(
                "id-1".to_string(),
                LayoutPosition {
                    x: 640,
                    y: 0,
                    primary: true,
                },
            )]),
            ..DeviceConfig::default()
        };
        let (mut runtime, _toasts) = layout_runtime(control.clone(), store.clone(), false);
        runtime.start(&config);
        runtime.handle(Command::ApplyLayout);
        let calls = control.layout_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0][0].x, 640);
        assert_eq!(calls[0][0].y, 0);
        assert!(calls[0][0].primary);
        assert_eq!(store.load_layout().unwrap().unwrap().mutations, 1);
    }

    #[test]
    fn apply_layout_without_a_primary_refuses_and_claims_nothing() {
        let (_dir, store) = runtime_store();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot(
                "id-1",
                "card0-DP-1",
                0.0,
                0.0,
                false,
                None,
            ))
            .with_store(store.clone()),
        );
        let config = DeviceConfig {
            layout_position: BTreeMap::from([(
                "id-1".to_string(),
                LayoutPosition {
                    x: 640,
                    y: 0,
                    primary: false,
                },
            )]),
            ..DeviceConfig::default()
        };
        let (mut runtime, toasts) = layout_runtime(control.clone(), store.clone(), false);
        runtime.start(&config);
        runtime.handle(Command::ApplyLayout);
        assert!(store.load_layout().unwrap().is_none());
        assert!(control.claim_checks().is_empty());
        assert!(control.layout_calls().is_empty());
        assert!(toasts.lock().unwrap().last().unwrap().contains("primary"));
    }

    #[test]
    fn apply_layout_command_restores_display_state() {
        let (_dir, store) = runtime_store();
        store
            .write_snapshot(&crate::session::Snapshot {
                source: "gamma".into(),
                lut: Some(gamma_lut()),
                last_value: 40,
                ..stale_snapshot("id-1", "card0-DP-1", 100, 40)
            })
            .unwrap();
        store
            .write_snapshot(&crate::session::Snapshot {
                source: "gamma".into(),
                lut: Some(gamma_lut()),
                last_value: 55,
                ..stale_snapshot("id-2", "card0-HDMI-1", 100, 55)
            })
            .unwrap();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1"), handle("id-2", "card0-HDMI-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot("id-1", "card0-DP-1", 0.0, 0.0, true, None))
            .with_snapshot(display_snapshot(
                "id-2",
                "card0-HDMI-1",
                1920.0,
                0.0,
                false,
                None,
            ))
            .with_store(store.clone()),
        );
        let (_root, config_root) = preferred_root();
        let clock = Arc::new(StdMutex::new(Now {
            unix: 100_000,
            minute: Minute(12 * 60),
        }));
        let mut runtime = runtime_with_root(control.clone(), store.clone(), Some(config_root))
            .with_clock(move || *clock.lock().unwrap());
        let config = DeviceConfig {
            layout_position: BTreeMap::from([
                (
                    "id-1".to_string(),
                    LayoutPosition {
                        x: 0,
                        y: 0,
                        primary: true,
                    },
                ),
                (
                    "id-2".to_string(),
                    LayoutPosition {
                        x: 1920,
                        y: 0,
                        primary: false,
                    },
                ),
            ]),
            ..DeviceConfig::default()
        };
        runtime.start(&config);
        runtime.handle(Command::Night(NightRequest::Toggle));
        let warm = Tint::from_kelvin(3500);
        let before = control.gamma_calls().len();
        runtime.handle(Command::ApplyLayout);
        let calls = control.layout_calls();
        assert_eq!(calls.len(), 1, "the configured layout must be written");
        assert_eq!(calls[0][0].x, 0);
        assert_eq!(calls[0][1].x, 1920);
        assert_eq!(store.load_layout().unwrap().unwrap().mutations, 1);
        let gamma = control.gamma_calls();
        let reapplied = &gamma[before..];
        let reasserted: HashSet<&str> = reapplied.iter().map(|(id, _, _)| id.as_str()).collect();
        assert!(
            reasserted.contains("id-1") && reasserted.contains("id-2"),
            "the layout write must re-assert every connected display"
        );
        assert!(reapplied.iter().filter(|(id, _, _)| id == "id-1").count() >= 1);
        assert!(reapplied.iter().filter(|(id, _, _)| id == "id-2").count() >= 1);
        let last = gamma.last().unwrap();
        assert_eq!(last.2, warm, "the reassert keeps the night tint");
        assert_eq!(runtime.night_payload()["state"], "active");
    }

    #[test]
    fn apply_layout_command_skips_the_restore_when_nothing_is_configured() {
        let (_dir, store) = runtime_store();
        store
            .write_snapshot(&crate::session::Snapshot {
                source: "gamma".into(),
                lut: Some(gamma_lut()),
                last_value: 40,
                ..stale_snapshot("id-1", "card0-DP-1", 100, 40)
            })
            .unwrap();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot("id-1", "card0-DP-1", 0.0, 0.0, true, None))
            .with_store(store.clone()),
        );
        let (mut runtime, _toasts) = layout_runtime(control.clone(), store.clone(), false);
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::ApplyLayout);
        assert!(control.layout_calls().is_empty());
        assert!(
            control.gamma_calls().is_empty(),
            "an empty configured layout must not re-assert any display"
        );
        assert!(
            control.tints().is_empty(),
            "an empty configured layout must not write a night tint"
        );
    }

    #[test]
    fn resident_start_and_kill_skip_layout_restore() {
        let (_dir, store) = runtime_store();
        store
            .claim_layout(&layout_capture(
                vec![PlacementRecord {
                    id: "id-1".into(),
                    connector: "card0-DP-1".into(),
                    x: 3840,
                    y: 0,
                    primary: true,
                }],
                vec![],
            ))
            .unwrap();
        store.touch_layout().unwrap();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot("id-1", "card0-DP-1", 0.0, 0.0, true, None))
            .with_store(store.clone()),
        );
        let (mut runtime, _toasts) = layout_runtime(control.clone(), store.clone(), true);
        runtime.start(&DeviceConfig::default());
        assert!(control.layout_calls().is_empty());
        assert!(control.claim_checks().is_empty());
        assert!(
            store.load_layout().unwrap().is_some(),
            "a resident host leaves the pending layout snapshot alone"
        );
        assert!(!runtime.handle(Command::Kill));
        assert!(control.layout_calls().is_empty());
        assert!(store.load_layout().unwrap().is_some());
    }

    #[test]
    fn reload_handoff_does_not_restore_the_layout_on_the_successor_start() {
        let (_dir, store) = runtime_store();
        let main = display_snapshot("id-1", "card0-DP-1", 0.0, 0.0, true, None);
        let secondary = display_snapshot("id-2", "card0-HDMI-1", 1920.0, 0.0, false, None);
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1"), handle("id-2", "card0-HDMI-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(main)
            .with_snapshot(secondary),
        );
        let mut predecessor = runtime_with_generation(control.clone(), store.clone(), "gen-a");
        predecessor.start(&DeviceConfig::default());
        predecessor.handle(Command::SetPrimary {
            display: "id-2".into(),
        });
        assert_eq!(control.layout_calls().len(), 1);
        assert_eq!(store.load_layout().unwrap().unwrap().mutations, 1);
        assert!(
            !predecessor.handle(Command::Handoff),
            "SIGHUP ends the loop"
        );
        drop(predecessor);
        assert!(
            store.load_layout().unwrap().unwrap().handoff,
            "SIGHUP must mark the layout handoff"
        );

        control.layout_calls.lock().unwrap().clear();
        let mut successor = runtime_with_generation(control.clone(), store.clone(), "gen-a");
        successor.start(&DeviceConfig::default());
        assert!(
            control.layout_calls().is_empty(),
            "the reload successor must adopt the layout without restoring it"
        );
        assert!(
            !store.load_layout().unwrap().unwrap().handoff,
            "the successor clears the layout handoff marker"
        );
        assert!(!successor.handle(Command::Kill));
        let restored = control.layout_calls();
        assert_eq!(
            restored.len(),
            1,
            "a real exit still restores the captured layout"
        );
        assert_eq!(restored[0][0].handle.id(), "id-1");
        assert!(restored[0][0].primary);
        assert_eq!(restored[0][1].handle.id(), "id-2");
        assert!(!restored[0][1].primary);
        assert!(
            store.load_layout().unwrap().is_none(),
            "the exit restore clears the snapshot"
        );
    }

    #[test]
    fn a_layout_capture_failure_aborts_before_the_write_with_the_operation_name() {
        let (_dir, store) = runtime_store();
        std::fs::create_dir_all(store.dir()).unwrap();
        std::fs::write(store.dir().join("layout"), b"blocked").unwrap();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot("id-1", "card0-DP-1", 0.0, 0.0, true, None))
            .with_modes(
                "id-1",
                vec![
                    display_mode(1, 1920, 1080, 60),
                    display_mode(2, 1280, 720, 60),
                ],
            )
            .with_store(store.clone()),
        );
        let (mut runtime, toasts) = layout_runtime(control.clone(), store.clone(), false);
        runtime.start(&DeviceConfig {
            notify_on_change: false,
            ..DeviceConfig::default()
        });
        let commands = [
            (
                Command::SetMode {
                    display: "id-1".into(),
                    token: None,
                    width: 1280,
                    height: 720,
                    refresh: Some(60),
                },
                "Mode not set",
            ),
            (
                Command::SetPrimary {
                    display: "id-1".into(),
                },
                "Primary not set",
            ),
            (
                Command::Arrange {
                    placements: vec![ArrangeRequest {
                        id: "id-1".into(),
                        x: 0,
                        y: 0,
                    }],
                    primary: None,
                },
                "Layout not applied",
            ),
        ];
        for (command, expected) in commands {
            runtime.handle(command);
            let bodies = toasts.lock().unwrap().clone();
            assert!(
                bodies.last().unwrap().contains(expected),
                "expected `{expected}` in {bodies:?}"
            );
        }
        assert!(control.mode_calls().is_empty());
        assert!(control.layout_calls().is_empty());
        assert!(store.load_layout().unwrap().is_none());
    }

    #[test]
    fn a_layout_mutation_failure_aborts_with_the_operation_name() {
        let (_dir, store) = runtime_store();
        store
            .claim_layout(&layout_capture(
                vec![PlacementRecord {
                    id: "id-1".into(),
                    connector: "card0-DP-1".into(),
                    x: 0,
                    y: 0,
                    primary: true,
                }],
                vec![],
            ))
            .unwrap();
        let path = store.dir().join("layout").join("layout.json");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let control = Arc::new(FakeControl::new(
            vec![handle("id-1", "card0-DP-1")],
            70,
            BrightnessSource::Ddc,
        ));
        let (runtime, toasts) = layout_runtime(control.clone(), store.clone(), false);
        assert!(!runtime.record_layout_mutation("Layout not applied"));
        let bodies = toasts.lock().unwrap().clone();
        assert!(
            bodies.last().unwrap().contains("Layout not applied")
                && bodies.last().unwrap().contains("could not be updated"),
            "{bodies:?}"
        );
        assert!(path.is_dir());
    }

    #[test]
    fn a_failed_layout_write_keeps_the_recorded_mutation() {
        let (_dir, store) = runtime_store();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot("id-1", "card0-DP-1", 0.0, 0.0, true, None))
            .with_store(store.clone()),
        );
        control.layout_fail.store(true, Ordering::SeqCst);
        let (mut runtime, toasts) = layout_runtime(control.clone(), store.clone(), false);
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::SetPrimary {
            display: "id-1".into(),
        });
        let snapshot = store.load_layout().unwrap().unwrap();
        assert_eq!(
            snapshot.mutations, 1,
            "the recorded mutation survives a refused hardware write"
        );
        assert!(toasts
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("Primary not set"));
    }

    #[test]
    fn layout_write_success_toasts_honor_notify_on_change() {
        for (notify_on_change, expected) in [(false, 0usize), (true, 2usize)] {
            let (_dir, store) = runtime_store();
            let control = Arc::new(
                FakeControl::new(
                    vec![handle("id-1", "card0-DP-1")],
                    70,
                    BrightnessSource::Ddc,
                )
                .with_snapshot(display_snapshot("id-1", "card0-DP-1", 0.0, 0.0, true, None))
                .with_modes(
                    "id-1",
                    vec![
                        display_mode(1, 1920, 1080, 60),
                        display_mode(2, 1280, 720, 60),
                    ],
                ),
            );
            let (mut runtime, toasts) = layout_runtime(control.clone(), store.clone(), false);
            runtime.start(&DeviceConfig {
                notify_on_change,
                ..DeviceConfig::default()
            });
            runtime.handle(Command::SetMode {
                display: "id-1".into(),
                token: None,
                width: 1280,
                height: 720,
                refresh: Some(60),
            });
            runtime.handle(Command::SetPrimary {
                display: "id-1".into(),
            });
            assert_eq!(
                toasts.lock().unwrap().len(),
                expected,
                "notify_on_change {notify_on_change}"
            );
            assert_eq!(control.mode_calls().len(), 1);
            assert_eq!(control.layout_calls().len(), 1);
        }
    }

    #[test]
    fn apply_layout_with_an_empty_config_map_claims_and_writes_nothing() {
        let (_dir, store) = runtime_store();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot("id-1", "card0-DP-1", 0.0, 0.0, true, None))
            .with_store(store.clone()),
        );
        let (mut runtime, toasts) = layout_runtime(control.clone(), store.clone(), false);
        runtime.start(&DeviceConfig::default());
        runtime.handle(Command::ApplyLayout);
        assert!(store.load_layout().unwrap().is_none());
        assert!(control.claim_checks().is_empty());
        assert!(control.layout_calls().is_empty());
        assert!(toasts.lock().unwrap().is_empty());
    }

    #[test]
    fn a_failed_layout_restore_notifies_and_keeps_the_snapshot() {
        let (_dir, store) = runtime_store();
        store
            .claim_layout(&layout_capture(
                vec![PlacementRecord {
                    id: "id-1".into(),
                    connector: "card0-DP-1".into(),
                    x: 100,
                    y: 0,
                    primary: true,
                }],
                vec![],
            ))
            .unwrap();
        store.touch_layout().unwrap();
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot(
                "id-1",
                "card0-DP-1",
                0.0,
                0.0,
                true,
                None,
            )),
        );
        control.layout_fail.store(true, Ordering::SeqCst);
        let (mut runtime, toasts) = layout_runtime(control.clone(), store.clone(), false);
        runtime.start(&DeviceConfig::default());
        let bodies = toasts.lock().unwrap().clone();
        assert!(
            bodies
                .iter()
                .any(|body| body.contains("Display layout could not be restored")),
            "{bodies:?}"
        );
        assert!(store.load_layout().unwrap().is_some());
    }

    #[test]
    fn a_running_query_releases_the_runtime_lock_so_a_kill_is_not_starved() {
        let (_dir, store) = runtime_store();
        let gate = Arc::new(SnapshotGate::new());
        let control = Arc::new(
            FakeControl::new(
                vec![handle("id-1", "card0-DP-1")],
                70,
                BrightnessSource::Ddc,
            )
            .with_snapshot(display_snapshot("id-1", "card0-DP-1", 0.0, 0.0, true, None))
            .with_snapshot_gate(gate.clone()),
        );
        let runtime: Arc<Mutex<Runtime<dyn MonitorControl>>> = Arc::new(Mutex::new(Runtime::new(
            control.clone(),
            store,
            Arc::new(NoLutProvider),
            |_title, _body| {},
            None,
            |_preferred| Ok(()),
            || false,
        )));
        let query = {
            let live = Arc::clone(&runtime);
            std::thread::spawn(move || live_query_from(&live, "layout"))
        };
        assert!(gate.wait_entered(), "the query must reach display I/O");
        let mut acquired = false;
        for _ in 0..100_000 {
            if let Ok(guard) = runtime.try_lock() {
                drop(guard);
                acquired = true;
                break;
            }
            std::thread::yield_now();
        }
        if acquired {
            assert!(
                !runtime.lock().unwrap().handle(Command::Kill),
                "the kill must be handled while the query is blocked"
            );
        }
        gate.release();
        let payload = query.join().expect("the query thread must finish");
        assert!(
            acquired,
            "the runtime mutex must be released before display I/O"
        );
        assert!(matches!(payload, ReadResult::HandledWithData(_)));
    }
}
