use super::capture::HEARTBEAT_INTERVAL;
use super::catalog::load_available_actions;
use super::hold::{Fire, HoldTimer};
use super::manager::{RegisteredHotkey, RegistrationScope};
use super::platform::PhysicalHotkeyState;
use super::reload;
use super::{HotkeyAction, HotkeyManager};
use crate::plugins::PluginManager;
use anyhow::{anyhow, Result};
use crossbeam_channel::{after, never, select, Receiver};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyEventReceiver, HotKeyState};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const INITIAL_BACKOFF: Duration = Duration::from_millis(500);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const REASSERT_SCHEDULE: &[Duration] = &[
    Duration::from_secs(10),
    Duration::from_secs(30),
    Duration::from_secs(60),
    Duration::from_secs(120),
    Duration::from_secs(240),
    Duration::from_secs(420),
];

type SharedPluginManager = Arc<Mutex<PluginManager>>;

pub fn start_hotkey_listener(plugin_manager: Arc<Mutex<PluginManager>>) -> Result<()> {
    start_listener(plugin_manager, RegistrationScope::All);
    Ok(())
}

pub(super) fn start_listener(plugin_manager: Arc<Mutex<PluginManager>>, scope: RegistrationScope) {
    let reload_rx = reload::subscribe();
    std::thread::spawn(move || {
        run_supervised(
            &mut |reload_rx| run_listener_once(&plugin_manager, reload_rx, scope),
            reload_rx,
            &mut std::thread::sleep,
            &mut mark_doctor_needed,
            || true,
        );
    });
}

fn mark_doctor_needed(reason: &str) {
    if let Err(write_err) = crate::doctor::trigger::mark_needed("hotkey_shadows", reason) {
        log::warn!("doctor trigger: mark_needed failed: {}", write_err);
    }
}

fn run_supervised(
    runner: &mut dyn FnMut(&Receiver<()>) -> Result<()>,
    reload_rx: Receiver<()>,
    sleeper: &mut dyn FnMut(Duration),
    trigger_doctor: &mut dyn FnMut(&str),
    mut should_continue: impl FnMut() -> bool,
) {
    let mut backoff = INITIAL_BACKOFF;
    while should_continue() {
        let outcome = runner(&reload_rx);
        let saturated = backoff == MAX_BACKOFF;
        match &outcome {
            Ok(()) => {
                log::error!(
                    "Hotkey listener exited without error; restarting in {}ms",
                    backoff.as_millis()
                );
            }
            Err(error) => {
                log::error!(
                    "Hotkey listener failed: {}; restarting in {}ms",
                    error,
                    backoff.as_millis()
                );
                if saturated {
                    trigger_doctor(&format!(
                        "hotkey listener still failing after backoff cap: {}",
                        error
                    ));
                }
            }
        }
        if !should_continue() {
            return;
        }
        sleeper(backoff);
        backoff = next_backoff(backoff);
    }
}

fn next_backoff(current: Duration) -> Duration {
    let doubled = current.saturating_mul(2);
    if doubled > MAX_BACKOFF {
        MAX_BACKOFF
    } else {
        doubled
    }
}

fn run_listener_once(
    plugin_manager: &SharedPluginManager,
    reload_rx: &Receiver<()>,
    scope: RegistrationScope,
) -> Result<()> {
    let manager = HotkeyManager::with_scope(scope)
        .map_err(|e| anyhow!("failed to create hotkey manager: {}", e))?;
    let physical_state = PhysicalHotkeyState::connect()
        .map_err(|e| anyhow!("failed to connect physical hotkey state: {}", e))?;
    HotkeyListenerLoop {
        manager,
        plugin_manager: plugin_manager.clone(),
        reload_rx,
        held_actions: HeldActions::default(),
        physical_state,
        last_heartbeat: Instant::now(),
        trace_session: 0,
        next_trace_session: 0,
        next_trace_sequence: 0,
        reassert_schedule: REASSERT_SCHEDULE,
        reassert_index: 0,
    }
    .run();
    Err(anyhow!("hotkey listener loop returned unexpectedly"))
}

struct HotkeyListenerLoop<'a> {
    manager: HotkeyManager,
    plugin_manager: SharedPluginManager,
    reload_rx: &'a Receiver<()>,
    held_actions: HeldActions,
    physical_state: PhysicalHotkeyState,
    last_heartbeat: Instant,
    trace_session: u64,
    next_trace_session: u64,
    next_trace_sequence: u64,
    reassert_schedule: &'static [Duration],
    reassert_index: usize,
}

impl<'a> HotkeyListenerLoop<'a> {
    fn register_initial_hotkeys(&mut self) {
        match self.reload_hotkeys() {
            Ok(()) => {
                qol_runtime::probe!("HOTKEY_LISTENER", "phase=initial-registration outcome=ok");
            }
            Err(error) => {
                qol_runtime::probe!(
                    "HOTKEY_LISTENER",
                    "phase=initial-registration outcome=error reason={}",
                    qol_runtime::probe::token(&error.to_string())
                );
                log::error!("Failed to register hotkeys: {}", error);
            }
        }
    }

    fn reload_hotkeys(&mut self) -> Result<()> {
        let config = self.manager.load_config()?;
        let available_actions = load_available_actions(&self.plugin_manager);
        self.manager.register_hotkeys(&config, &available_actions)
    }

    fn run(mut self) {
        self.register_initial_hotkeys();
        let hotkey_receiver: &GlobalHotKeyEventReceiver = GlobalHotKeyEvent::receiver();
        qol_runtime::probe!("HOTKEY_LISTENER", "phase=select-entered");

        loop {
            let physical_state_rx: Receiver<Instant> =
                if self.held_actions.is_empty() && !super::platform::POLL_WHILE_IDLE {
                    never()
                } else {
                    after(super::platform::POLL_INTERVAL)
                };
            let hold_rx: Receiver<Instant> = match self.held_actions.next_deadline() {
                Some(deadline) => after(deadline.saturating_duration_since(Instant::now())),
                None => never(),
            };
            let reassert_rx: Receiver<Instant> =
                match self.reassert_schedule.get(self.reassert_index) {
                    Some(offset) => after(*offset),
                    None => never(),
                };
            select! {
                recv(self.reload_rx) -> reload => {
                    if reload.is_err() {
                        break;
                    }
                    self.drain_reload_signals();
                    self.handle_reload();
                }
                recv(hotkey_receiver) -> event => {
                    let Ok(event) = event else { break };
                    self.handle_hotkey_event(event);
                }
                recv(physical_state_rx) -> _ => self.poll_physical_state(),
                recv(hold_rx) -> _ => self.fire_due_holds(),
                recv(reassert_rx) -> _ => {
                    self.reassert_index += 1;
                    self.reassert_hotkeys();
                }
            }
        }
        self.stop_held_actions(DispatchSource::ForcedStop);
    }

    fn reassert_hotkeys(&mut self) {
        qol_runtime::probe!("HOTKEY_LISTENER", "phase=reassert");
        match self.manager.reassert_all() {
            Ok(()) => log::info!("Re-asserted hotkey grabs after the desktop settled"),
            Err(error) => log::error!("Failed to re-assert hotkey grabs: {}", error),
        }
    }

    fn drain_reload_signals(&self) {
        while self.reload_rx.try_recv().is_ok() {}
    }

    fn handle_reload(&mut self) {
        qol_runtime::probe!("HOTKEY_LISTENER", "phase=reload-received");
        log::info!("Reloading hotkeys...");
        self.stop_held_actions(DispatchSource::ForcedStop);
        match self.reload_hotkeys() {
            Ok(()) => {
                qol_runtime::probe!("HOTKEY_LISTENER", "phase=reload-done outcome=ok");
                log::info!("Hotkeys reloaded successfully");
            }
            Err(error) => {
                qol_runtime::probe!(
                    "HOTKEY_LISTENER",
                    "phase=reload-done outcome=error reason={}",
                    qol_runtime::probe::token(&error.to_string())
                );
                log::error!("Failed to register hotkeys: {}", error);
            }
        }
    }

    fn handle_hotkey_event(&mut self, event: GlobalHotKeyEvent) {
        qol_runtime::probe!(
            "HOTKEY_LISTENER",
            "phase=event-received id={} state={}",
            event.id(),
            match event.state {
                global_hotkey::HotKeyState::Pressed => "pressed",
                global_hotkey::HotKeyState::Released => "released",
            }
        );
        let action = self.manager.get_registration(&event).cloned();
        let Some(dispatch) = self
            .held_actions
            .handle_event(event, action.as_ref(), Instant::now())
        else {
            return;
        };
        self.dispatch(dispatch, DispatchSource::GlobalEvent, "event");
        self.finish_trace_session_if_idle();
    }

    fn fire_due_holds(&mut self) {
        for dispatch in self.held_actions.expire(Instant::now()) {
            self.dispatch(dispatch, DispatchSource::HoldTimer, "held");
        }
        self.finish_trace_session_if_idle();
    }

    fn poll_physical_state(&mut self) {
        let snapshot = match self.physical_state.snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                log::warn!("Physical hotkey state query failed: {error}");
                self.stop_held_actions(DispatchSource::ForcedStop);
                return;
            }
        };
        let dispatches = if snapshot.supports_reconciliation() {
            self.held_actions.reconcile_physical(
                self.manager.held_registrations(),
                |registration| {
                    registration
                        .physical_chord
                        .as_ref()
                        .is_some_and(|chord| snapshot.chord_is_pressed(chord))
                },
                Instant::now(),
            )
        } else {
            Vec::new()
        };
        let key_summary = (!dispatches.is_empty()).then(|| snapshot.trace_summary());
        for dispatch in dispatches {
            self.log_physical_transition(&dispatch);
            self.dispatch(
                dispatch,
                DispatchSource::PhysicalState,
                key_summary.as_deref().unwrap_or("unknown"),
            );
        }
        self.finish_trace_session_if_idle();
        if self.held_actions.is_empty() || self.last_heartbeat.elapsed() < HEARTBEAT_INTERVAL {
            return;
        }
        self.last_heartbeat = Instant::now();
        for dispatch in self.held_actions.heartbeat_dispatches() {
            self.dispatch(dispatch, DispatchSource::Heartbeat, "unchanged");
        }
    }

    fn log_physical_transition(&self, dispatch: &HotkeyDispatch) {
        let HotkeyDispatch::Continuous { action, phase } = dispatch else {
            return;
        };
        log::info!(
            "Continuous hotkey chord transition: {}::{} phase={} source=physical-state",
            action.plugin_uid.as_str(),
            action.action,
            phase.as_str()
        );
    }

    fn stop_held_actions(&mut self, source: DispatchSource) {
        for dispatch in self.held_actions.stop_all() {
            self.dispatch(dispatch, source, "unavailable");
        }
        self.finish_trace_session_if_idle();
    }

    fn dispatch(&mut self, dispatch: HotkeyDispatch, source: DispatchSource, _keys: &str) {
        let action = dispatch.action();

        let plugin_id = match self.plugin_manager.lock() {
            Ok(manager) => manager
                .identity_index()
                .display_for(&action.plugin_uid)
                .map(|d| d.id.as_str().to_owned()),
            Err(_) => {
                log::error!("hotkey dispatch: plugin manager lock failed");
                return;
            }
        };

        let Some(plugin_id) = plugin_id else {
            log::warn!(
                "hotkey dispatch: no plugin found for uid {}",
                action.plugin_uid.as_str()
            );
            return;
        };

        match dispatch {
            HotkeyDispatch::OneShot(action) => {
                log::info!(
                    "Hotkey triggered: {}::{}",
                    action.plugin_uid.as_str(),
                    action.action
                );
                qol_runtime::probe!(
                    "HOTKEY_DISPATCH",
                    "source={} kind=one-shot uid={} action={}",
                    source.as_str(),
                    qol_runtime::probe::token(action.plugin_uid.as_str()),
                    qol_runtime::probe::token(&action.action)
                );
                crate::plugins::action_executor::execute_action(
                    &self.plugin_manager,
                    &plugin_id,
                    &action.action,
                );
            }
            HotkeyDispatch::Hold(action) => {
                if let Err(error) = self.manager.release_active_grab() {
                    log::error!(
                        "Hold hotkey could not hand keyboard ownership to the window manager: {error}"
                    );
                }
                log::info!(
                    "Hotkey held: {}::{}",
                    action.plugin_uid.as_str(),
                    action.action
                );
                qol_runtime::probe!(
                    "HOTKEY_DISPATCH",
                    "source={} kind=hold uid={} action={}",
                    source.as_str(),
                    qol_runtime::probe::token(action.plugin_uid.as_str()),
                    qol_runtime::probe::token(&action.action)
                );
                crate::plugins::action_executor::execute_action(
                    &self.plugin_manager,
                    &plugin_id,
                    &action.action,
                );
            }
            HotkeyDispatch::Continuous { action, phase } => {
                if phase == ContinuousPhase::Start {
                    match self.manager.release_active_grab() {
                        Ok(()) => {
                            #[cfg(debug_assertions)]
                            qol_runtime::probe!(
                                "WINACT_HOTKEY",
                                "phase=grab-handoff outcome=ok owner=window-manager"
                            );
                        }
                        Err(error) => log::error!(
                            "Continuous hotkey could not hand keyboard ownership to the window manager: {error}"
                        ),
                    }
                }
                let (session, sequence) = self.trace_ids(phase);
                #[cfg(debug_assertions)]
                let active = self.held_actions.trace_active_directions();
                if phase != ContinuousPhase::Heartbeat {
                    #[cfg(debug_assertions)]
                    qol_runtime::probe!(
                        "WINACT_HOTKEY",
                        "session={} seq={} source={} phase={} direction={} active={} keys={}",
                        session,
                        sequence,
                        source.as_str(),
                        phase.as_str(),
                        action
                            .action
                            .strip_prefix("glide-")
                            .unwrap_or(&action.action),
                        active,
                        _keys
                    );
                }
                if phase == ContinuousPhase::Heartbeat {
                    log::trace!(
                        "Continuous hotkey phase: {}::{} phase={}",
                        action.plugin_uid.as_str(),
                        action.action,
                        phase.as_str()
                    );
                } else {
                    log::info!(
                        "Continuous hotkey phase: {}::{} phase={}",
                        action.plugin_uid.as_str(),
                        action.action,
                        phase.as_str()
                    );
                }
                crate::plugins::action_executor::execute_action_with_input(
                    &self.plugin_manager,
                    &plugin_id,
                    &action.action,
                    serde_json::json!({
                        "phase": phase.as_str(),
                        "trace_session": session,
                        "trace_seq": sequence,
                        "trace_source": source.as_str(),
                    }),
                );
            }
        }
    }

    fn trace_ids(&mut self, phase: ContinuousPhase) -> (u64, u64) {
        if self.trace_session == 0 {
            self.next_trace_session = self.next_trace_session.wrapping_add(1).max(1);
            self.trace_session = self.next_trace_session;
        }
        if phase == ContinuousPhase::Heartbeat {
            return (self.trace_session, 0);
        }
        self.next_trace_sequence = self.next_trace_sequence.wrapping_add(1).max(1);
        (self.trace_session, self.next_trace_sequence)
    }

    fn finish_trace_session_if_idle(&mut self) {
        if self.held_actions.is_empty() {
            self.trace_session = 0;
        }
    }
}

#[derive(Clone, Copy)]
enum DispatchSource {
    GlobalEvent,
    PhysicalState,
    Heartbeat,
    ForcedStop,
    HoldTimer,
}

impl DispatchSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::GlobalEvent => "global-event",
            Self::PhysicalState => "physical-state",
            Self::Heartbeat => "heartbeat",
            Self::ForcedStop => "forced-stop",
            Self::HoldTimer => "hold-timer",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContinuousPhase {
    Start,
    Heartbeat,
    Stop,
}

impl ContinuousPhase {
    fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Heartbeat => "heartbeat",
            Self::Stop => "stop",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HotkeyDispatch {
    OneShot(HotkeyAction),
    Hold(HotkeyAction),
    Continuous {
        action: HotkeyAction,
        phase: ContinuousPhase,
    },
}

impl HotkeyDispatch {
    fn action(&self) -> &HotkeyAction {
        match self {
            Self::OneShot(action) | Self::Hold(action) | Self::Continuous { action, .. } => action,
        }
    }
}

enum HeldState {
    Continuous,
    Timed(HoldTimer),
}

struct Held {
    registration: RegisteredHotkey,
    state: HeldState,
}

#[derive(Default)]
struct HeldActions {
    actions: HashMap<u32, Held>,
}

impl Held {
    fn is_stale(&self, now: Instant) -> bool {
        matches!(&self.state, HeldState::Timed(timer) if timer.is_stale(now))
    }
}

impl HeldActions {
    fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    #[cfg(debug_assertions)]
    fn trace_active_directions(&self) -> String {
        let mut directions = self
            .actions
            .values()
            .filter_map(|held| held.registration.action.action.strip_prefix("glide-"))
            .collect::<Vec<_>>();
        directions.sort_unstable();
        if directions.is_empty() {
            "none".into()
        } else {
            directions.join(",")
        }
    }

    fn start(
        &mut self,
        id: u32,
        registration: RegisteredHotkey,
        now: Instant,
    ) -> Option<HotkeyDispatch> {
        if let Some(plan) = registration.hold.clone() {
            self.actions.insert(
                id,
                Held {
                    registration,
                    state: HeldState::Timed(HoldTimer::start(plan, now)),
                },
            );
            return None;
        }
        let action = registration.action.clone();
        self.actions.insert(
            id,
            Held {
                registration,
                state: HeldState::Continuous,
            },
        );
        Some(HotkeyDispatch::Continuous {
            action,
            phase: ContinuousPhase::Start,
        })
    }

    fn finish(held: Held, now: Instant) -> Option<HotkeyDispatch> {
        match held.state {
            HeldState::Continuous => Some(HotkeyDispatch::Continuous {
                action: held.registration.action,
                phase: ContinuousPhase::Stop,
            }),
            HeldState::Timed(timer) => timer.release(now).map(dispatch_of),
        }
    }

    fn handle_event(
        &mut self,
        event: GlobalHotKeyEvent,
        registered_action: Option<&RegisteredHotkey>,
        now: Instant,
    ) -> Option<HotkeyDispatch> {
        match event.state {
            HotKeyState::Pressed => {
                let registration = registered_action?.clone();
                if registration.hold.is_none() && !registration.action.continuous {
                    return Some(HotkeyDispatch::OneShot(registration.action));
                }
                if self
                    .actions
                    .get(&event.id)
                    .is_some_and(|held| !held.is_stale(now))
                {
                    return None;
                }
                self.start(event.id, registration, now)
            }
            HotKeyState::Released => {
                let held = self.actions.remove(&event.id)?;
                Self::finish(held, now)
            }
        }
    }

    fn reconcile_physical<'a>(
        &mut self,
        registrations: impl Iterator<Item = (u32, &'a RegisteredHotkey)>,
        is_pressed: impl Fn(&RegisteredHotkey) -> bool,
        now: Instant,
    ) -> Vec<HotkeyDispatch> {
        let mut dispatches = Vec::new();
        let mut pressed_ids = HashSet::new();
        for (id, registration) in registrations {
            if !is_pressed(registration) {
                continue;
            }
            pressed_ids.insert(id);
            if self.actions.contains_key(&id) {
                continue;
            }
            dispatches.extend(self.start(id, registration.clone(), now));
        }
        let released = self
            .actions
            .keys()
            .filter(|id| !pressed_ids.contains(id))
            .copied()
            .collect::<Vec<_>>();
        for id in released {
            let Some(held) = self.actions.remove(&id) else {
                continue;
            };
            dispatches.extend(Self::finish(held, now));
        }
        dispatches
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.actions
            .values()
            .filter_map(|held| match &held.state {
                HeldState::Timed(timer) => timer.deadline(),
                HeldState::Continuous => None,
            })
            .min()
    }

    fn expire(&mut self, now: Instant) -> Vec<HotkeyDispatch> {
        self.actions
            .values_mut()
            .filter_map(|held| match &mut held.state {
                HeldState::Timed(timer) => timer.expire(now).map(dispatch_of),
                HeldState::Continuous => None,
            })
            .collect()
    }

    fn heartbeat_dispatches(&self) -> Vec<HotkeyDispatch> {
        self.actions
            .values()
            .filter(|held| matches!(held.state, HeldState::Continuous))
            .map(|held| HotkeyDispatch::Continuous {
                action: held.registration.action.clone(),
                phase: ContinuousPhase::Heartbeat,
            })
            .collect()
    }

    fn stop_all(&mut self) -> Vec<HotkeyDispatch> {
        self.actions
            .drain()
            .filter_map(|(_, held)| match held.state {
                HeldState::Continuous => Some(HotkeyDispatch::Continuous {
                    action: held.registration.action,
                    phase: ContinuousPhase::Stop,
                }),
                HeldState::Timed(_) => None,
            })
            .collect()
    }
}

fn dispatch_of(fire: Fire) -> HotkeyDispatch {
    match fire {
        Fire::Tap(target) => HotkeyDispatch::OneShot(target.hotkey_action()),
        Fire::Hold(target) => HotkeyDispatch::Hold(target.hotkey_action()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::PluginUid;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn registration(name: &str, continuous: bool) -> RegisteredHotkey {
        RegisteredHotkey {
            action: HotkeyAction {
                plugin_uid: PluginUid::new("uid-window-actions"),
                action: name.to_string(),
                continuous,
            },
            hold: None,
            physical_chord: crate::hotkeys::capture::parse_combo("Ctrl+Shift+Super+Left"),
        }
    }

    fn timed(name: &str, tap: Option<&str>, hold_ms: u32) -> RegisteredHotkey {
        let target = |action: &str| crate::hotkeys::hold::Target {
            plugin_uid: PluginUid::new("uid-shot"),
            action: action.to_string(),
        };
        let plan = crate::hotkeys::hold::HoldPlan {
            hold_ms,
            hold: target(&format!("hold-{name}")),
            tap: tap.map(|tap| target(&format!("{tap}-{name}"))),
        };
        RegisteredHotkey {
            action: plan.primary().hotkey_action(),
            hold: Some(plan),
            physical_chord: crate::hotkeys::capture::parse_combo("Shift+Super+S"),
        }
    }

    fn event(id: u32, state: HotKeyState) -> GlobalHotKeyEvent {
        GlobalHotKeyEvent { id, state }
    }

    #[test]
    fn one_shot_actions_dispatch_only_on_press() {
        let mut held = HeldActions::default();
        let now = Instant::now();
        let center = registration("center", false);

        assert_eq!(
            held.handle_event(event(1, HotKeyState::Pressed), Some(&center), now),
            Some(HotkeyDispatch::OneShot(center.action))
        );
        assert_eq!(
            held.handle_event(event(1, HotKeyState::Released), None, now),
            None
        );
        assert!(held.is_empty());
    }

    #[test]
    fn continuous_actions_dispatch_start_heartbeats_and_stop() {
        let mut held = HeldActions::default();
        let now = Instant::now();
        let glide = registration("glide-left", true);

        assert_eq!(
            held.handle_event(event(7, HotKeyState::Pressed), Some(&glide), now),
            Some(HotkeyDispatch::Continuous {
                action: glide.action.clone(),
                phase: ContinuousPhase::Start,
            })
        );
        assert_eq!(
            held.heartbeat_dispatches(),
            vec![HotkeyDispatch::Continuous {
                action: glide.action.clone(),
                phase: ContinuousPhase::Heartbeat,
            }]
        );
        assert_eq!(
            held.handle_event(event(7, HotKeyState::Released), Some(&glide), now),
            Some(HotkeyDispatch::Continuous {
                action: glide.action,
                phase: ContinuousPhase::Stop,
            })
        );
        assert!(held.is_empty());
    }

    #[test]
    fn continuous_actions_ignore_duplicate_press_events() {
        let mut held = HeldActions::default();
        let now = Instant::now();
        let glide = registration("glide-right", true);

        assert!(held
            .handle_event(event(9, HotKeyState::Pressed), Some(&glide), now)
            .is_some());
        assert_eq!(
            held.handle_event(event(9, HotKeyState::Pressed), Some(&glide), now),
            None
        );
        assert_eq!(
            held.heartbeat_dispatches(),
            vec![HotkeyDispatch::Continuous {
                action: glide.action,
                phase: ContinuousPhase::Heartbeat,
            }]
        );
    }

    #[test]
    fn physical_chord_break_stops_action_when_release_event_is_missing() {
        let mut held = HeldActions::default();
        let now = Instant::now();
        let glide = registration("glide-left", true);
        held.handle_event(event(7, HotKeyState::Pressed), Some(&glide), now);

        assert_eq!(
            held.reconcile_physical(std::iter::empty(), |_| false, now),
            vec![HotkeyDispatch::Continuous {
                action: glide.action,
                phase: ContinuousPhase::Stop,
            }]
        );
        assert!(held.is_empty());
    }

    #[test]
    fn physical_reconciliation_starts_new_chord_during_existing_hold() {
        let mut held = HeldActions::default();
        let now = Instant::now();
        let right = registration("glide-right", true);
        let left = registration("glide-left", true);
        held.handle_event(event(9, HotKeyState::Pressed), Some(&right), now);

        assert_eq!(
            held.reconcile_physical([(9, &right), (7, &left)].into_iter(), |_| true, now),
            vec![HotkeyDispatch::Continuous {
                action: left.action,
                phase: ContinuousPhase::Start,
            }]
        );
    }

    #[test]
    fn physical_reconciliation_recovers_a_missed_initial_press() {
        let mut held = HeldActions::default();
        let now = Instant::now();
        let left = registration("glide-left", true);

        assert_eq!(
            held.reconcile_physical([(7, &left)].into_iter(), |_| true, now),
            vec![HotkeyDispatch::Continuous {
                action: left.action,
                phase: ContinuousPhase::Start,
            }]
        );
        assert!(!held.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_polls_physical_state_only_while_an_action_is_held() {
        const { assert!(!super::super::platform::POLL_WHILE_IDLE) };
        assert_eq!(super::super::platform::POLL_INTERVAL.as_millis(), 8);
    }

    #[test]
    fn physical_reconciliation_starts_replacement_before_stopping_old_chord() {
        let mut held = HeldActions::default();
        let now = Instant::now();
        let right = registration("glide-right", true);
        let left = registration("glide-left", true);
        held.handle_event(event(9, HotKeyState::Pressed), Some(&right), now);

        assert_eq!(
            held.reconcile_physical(
                [(9, &right), (7, &left)].into_iter(),
                |registration| registration.action.action == "glide-left",
                now
            ),
            vec![
                HotkeyDispatch::Continuous {
                    action: left.action,
                    phase: ContinuousPhase::Start,
                },
                HotkeyDispatch::Continuous {
                    action: right.action,
                    phase: ContinuousPhase::Stop,
                },
            ]
        );
    }

    #[test]
    fn stop_all_releases_every_held_continuous_action() {
        let mut held = HeldActions::default();
        let now = Instant::now();
        let left = registration("glide-left", true);
        let up = registration("glide-up", true);
        held.handle_event(event(1, HotKeyState::Pressed), Some(&left), now);
        held.handle_event(event(2, HotKeyState::Pressed), Some(&up), now);

        let mut stopped = held.stop_all();
        stopped.sort_by(|a, b| a.action().action.cmp(&b.action().action));

        assert_eq!(
            stopped,
            vec![
                HotkeyDispatch::Continuous {
                    action: left.action,
                    phase: ContinuousPhase::Stop,
                },
                HotkeyDispatch::Continuous {
                    action: up.action,
                    phase: ContinuousPhase::Stop,
                },
            ]
        );
        assert!(held.is_empty());
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn tap_of(name: &str) -> HotkeyDispatch {
        HotkeyDispatch::OneShot(timed_action(&format!("tap-{name}")))
    }

    fn hold_of(name: &str) -> HotkeyDispatch {
        HotkeyDispatch::Hold(timed_action(&format!("hold-{name}")))
    }

    fn timed_action(action: &str) -> HotkeyAction {
        HotkeyAction {
            plugin_uid: PluginUid::new("uid-shot"),
            action: action.to_string(),
            continuous: false,
        }
    }

    #[test]
    fn a_hold_press_dispatches_nothing_and_arms_a_deadline() {
        let mut held = HeldActions::default();
        let t0 = Instant::now();
        let shot = timed("s", Some("tap"), 1000);

        assert_eq!(
            held.handle_event(event(1, HotKeyState::Pressed), Some(&shot), t0),
            None
        );
        assert!(!held.is_empty());
        assert_eq!(held.next_deadline(), Some(t0 + ms(1000)));
    }

    #[test]
    fn a_release_before_the_deadline_runs_the_tap() {
        let mut held = HeldActions::default();
        let t0 = Instant::now();
        let shot = timed("s", Some("tap"), 1000);
        held.handle_event(event(1, HotKeyState::Pressed), Some(&shot), t0);

        assert_eq!(
            held.handle_event(event(1, HotKeyState::Released), None, t0 + ms(140)),
            Some(tap_of("s"))
        );
        assert!(held.is_empty());
        assert_eq!(held.next_deadline(), None);
    }

    #[test]
    fn a_hold_only_chord_released_early_runs_nothing() {
        let mut held = HeldActions::default();
        let t0 = Instant::now();
        let shot = timed("s", None, 1000);
        held.handle_event(event(1, HotKeyState::Pressed), Some(&shot), t0);

        assert_eq!(
            held.handle_event(event(1, HotKeyState::Released), None, t0 + ms(300)),
            None
        );
        assert!(held.is_empty());
    }

    #[test]
    fn reaching_the_deadline_runs_the_hold_once_and_the_release_runs_nothing() {
        let mut held = HeldActions::default();
        let t0 = Instant::now();
        let shot = timed("s", Some("tap"), 1000);
        held.handle_event(event(1, HotKeyState::Pressed), Some(&shot), t0);

        assert_eq!(held.expire(t0 + ms(999)), Vec::new());
        assert_eq!(held.expire(t0 + ms(1000)), vec![hold_of("s")]);
        assert_eq!(held.expire(t0 + ms(2000)), Vec::new());
        assert_eq!(held.next_deadline(), None);
        assert_eq!(
            held.handle_event(event(1, HotKeyState::Released), None, t0 + ms(2500)),
            None
        );
        assert!(held.is_empty());
    }

    #[test]
    fn a_repeated_press_event_keeps_the_first_deadline() {
        let mut held = HeldActions::default();
        let t0 = Instant::now();
        let shot = timed("s", Some("tap"), 1000);
        held.handle_event(event(1, HotKeyState::Pressed), Some(&shot), t0);

        assert_eq!(
            held.handle_event(event(1, HotKeyState::Pressed), Some(&shot), t0 + ms(500)),
            None
        );
        assert_eq!(held.next_deadline(), Some(t0 + ms(1000)));
    }

    #[test]
    fn heartbeats_skip_a_pending_hold_and_still_cover_continuous_actions() {
        let mut held = HeldActions::default();
        let now = Instant::now();
        let shot = timed("s", Some("tap"), 1000);
        let glide = registration("glide-left", true);
        held.handle_event(event(1, HotKeyState::Pressed), Some(&shot), now);
        assert_eq!(held.heartbeat_dispatches(), Vec::new());

        held.handle_event(event(2, HotKeyState::Pressed), Some(&glide), now);
        assert_eq!(
            held.heartbeat_dispatches(),
            vec![HotkeyDispatch::Continuous {
                action: glide.action,
                phase: ContinuousPhase::Heartbeat,
            }]
        );
    }

    #[test]
    fn a_forced_stop_cancels_a_pending_hold_without_running_anything() {
        let mut held = HeldActions::default();
        let t0 = Instant::now();
        let shot = timed("s", Some("tap"), 1000);
        held.handle_event(event(1, HotKeyState::Pressed), Some(&shot), t0);

        assert_eq!(held.stop_all(), Vec::new());
        assert!(held.is_empty());
        assert_eq!(held.expire(t0 + ms(5000)), Vec::new());
        assert_eq!(
            held.handle_event(event(1, HotKeyState::Released), None, t0 + ms(5100)),
            None
        );
    }

    #[test]
    fn a_missed_release_is_found_by_the_physical_poll_and_runs_the_tap() {
        let mut held = HeldActions::default();
        let t0 = Instant::now();
        let shot = timed("s", Some("tap"), 1000);
        held.handle_event(event(1, HotKeyState::Pressed), Some(&shot), t0);

        assert_eq!(
            held.reconcile_physical([(1, &shot)].into_iter(), |_| false, t0 + ms(200)),
            vec![tap_of("s")]
        );
        assert!(held.is_empty());
    }

    #[test]
    fn a_missed_release_after_the_hold_ran_stays_silent() {
        let mut held = HeldActions::default();
        let t0 = Instant::now();
        let shot = timed("s", Some("tap"), 500);
        held.handle_event(event(1, HotKeyState::Pressed), Some(&shot), t0);
        held.expire(t0 + ms(500));

        assert_eq!(
            held.reconcile_physical([(1, &shot)].into_iter(), |_| false, t0 + ms(900)),
            Vec::new()
        );
        assert!(held.is_empty());
    }

    #[test]
    fn a_missed_initial_press_arms_the_deadline_from_the_poll() {
        let mut held = HeldActions::default();
        let t0 = Instant::now();
        let shot = timed("s", Some("tap"), 1000);

        assert_eq!(
            held.reconcile_physical([(1, &shot)].into_iter(), |_| true, t0),
            Vec::new()
        );
        assert_eq!(held.next_deadline(), Some(t0 + ms(1000)));
        assert_eq!(
            held.reconcile_physical([(1, &shot)].into_iter(), |_| true, t0 + ms(400)),
            Vec::new()
        );
        assert_eq!(held.next_deadline(), Some(t0 + ms(1000)));
    }

    #[test]
    fn two_hold_chords_interleaved_keep_separate_deadlines() {
        let mut held = HeldActions::default();
        let t0 = Instant::now();
        let first = timed("s", Some("tap"), 1000);
        let second = timed("r", Some("tap"), 300);
        held.handle_event(event(1, HotKeyState::Pressed), Some(&first), t0);
        held.handle_event(event(2, HotKeyState::Pressed), Some(&second), t0 + ms(100));

        assert_eq!(held.next_deadline(), Some(t0 + ms(400)));
        assert_eq!(held.expire(t0 + ms(400)), vec![hold_of("r")]);
        assert_eq!(held.next_deadline(), Some(t0 + ms(1000)));
        assert_eq!(
            held.handle_event(event(1, HotKeyState::Released), None, t0 + ms(600)),
            Some(tap_of("s"))
        );
        assert_eq!(
            held.handle_event(event(2, HotKeyState::Released), None, t0 + ms(700)),
            None
        );
        assert!(held.is_empty());
    }

    #[test]
    fn a_press_after_a_missed_release_starts_over_once_the_fired_hold_is_stale() {
        let mut held = HeldActions::default();
        let t0 = Instant::now();
        let shot = timed("s", Some("tap"), 500);
        held.handle_event(event(1, HotKeyState::Pressed), Some(&shot), t0);
        held.expire(t0 + ms(500));

        assert_eq!(
            held.handle_event(event(1, HotKeyState::Pressed), Some(&shot), t0 + ms(2000)),
            None
        );
        assert_eq!(held.next_deadline(), None);

        let later = t0 + ms(70_000);
        assert_eq!(
            held.handle_event(event(1, HotKeyState::Pressed), Some(&shot), later),
            None
        );
        assert_eq!(held.next_deadline(), Some(later + ms(500)));
    }

    #[test]
    fn a_plain_one_shot_beside_a_hold_chord_still_fires_on_press() {
        let mut held = HeldActions::default();
        let t0 = Instant::now();
        let center = registration("center", false);
        let shot = timed("s", Some("tap"), 1000);
        held.handle_event(event(1, HotKeyState::Pressed), Some(&shot), t0);

        assert_eq!(
            held.handle_event(event(2, HotKeyState::Pressed), Some(&center), t0),
            Some(HotkeyDispatch::OneShot(center.action))
        );
    }

    #[test]
    fn next_backoff_doubles_until_cap() {
        let cases = [
            (Duration::from_millis(500), Duration::from_secs(1)),
            (Duration::from_secs(1), Duration::from_secs(2)),
            (Duration::from_secs(2), Duration::from_secs(4)),
            (Duration::from_secs(15), Duration::from_secs(30)),
            (Duration::from_secs(30), Duration::from_secs(30)),
            (Duration::from_secs(60), Duration::from_secs(30)),
        ];
        for (input, expected) in cases {
            assert_eq!(next_backoff(input), expected, "input: {:?}", input);
        }
    }

    fn no_op_trigger() -> impl FnMut(&str) {
        |_reason: &str| {}
    }

    #[test]
    fn supervisor_restarts_runner_after_failure() {
        let attempts = AtomicU32::new(0);
        let sleeps: Mutex<Vec<Duration>> = Mutex::new(Vec::new());
        let max_attempts = 3;

        let (_tx, rx) = crossbeam_channel::unbounded::<()>();
        let mut runner = |_rx: &Receiver<()>| -> Result<()> {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(anyhow!("simulated transient failure"))
        };
        let mut sleeper = |d: Duration| {
            sleeps.lock().unwrap().push(d);
        };
        let mut trigger = no_op_trigger();
        let should_continue = || attempts.load(Ordering::SeqCst) < max_attempts;

        run_supervised(&mut runner, rx, &mut sleeper, &mut trigger, should_continue);

        assert_eq!(attempts.load(Ordering::SeqCst), max_attempts);
        let recorded = sleeps.lock().unwrap();
        assert_eq!(recorded.len(), (max_attempts - 1) as usize);
        assert_eq!(recorded[0], INITIAL_BACKOFF);
        assert_eq!(recorded[1], next_backoff(INITIAL_BACKOFF));
    }

    #[test]
    fn supervisor_restarts_when_runner_returns_ok() {
        let attempts = AtomicU32::new(0);
        let sleeps: Mutex<Vec<Duration>> = Mutex::new(Vec::new());
        let max_attempts = 3;

        let (_tx, rx) = crossbeam_channel::unbounded::<()>();
        let mut runner = |_rx: &Receiver<()>| -> Result<()> {
            attempts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        };
        let mut sleeper = |d: Duration| {
            sleeps.lock().unwrap().push(d);
        };
        let mut trigger = no_op_trigger();
        let should_continue = || attempts.load(Ordering::SeqCst) < max_attempts;

        run_supervised(&mut runner, rx, &mut sleeper, &mut trigger, should_continue);

        assert_eq!(
            attempts.load(Ordering::SeqCst),
            max_attempts,
            "runner returning Ok must still trigger a restart"
        );
        let recorded = sleeps.lock().unwrap();
        assert_eq!(recorded.len(), (max_attempts - 1) as usize);
    }

    #[test]
    fn supervisor_caps_backoff_at_max() {
        let attempts = AtomicU32::new(0);
        let sleeps: Mutex<Vec<Duration>> = Mutex::new(Vec::new());
        let total_attempts = 12;

        let (_tx, rx) = crossbeam_channel::unbounded::<()>();
        let mut runner = |_rx: &Receiver<()>| -> Result<()> {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(anyhow!("always fail"))
        };
        let mut sleeper = |d: Duration| {
            sleeps.lock().unwrap().push(d);
        };
        let mut trigger = no_op_trigger();
        let should_continue = || attempts.load(Ordering::SeqCst) < total_attempts;

        run_supervised(&mut runner, rx, &mut sleeper, &mut trigger, should_continue);

        let recorded = sleeps.lock().unwrap();
        assert_eq!(recorded.len(), (total_attempts - 1) as usize);
        let last_third = &recorded[recorded.len().saturating_sub(3)..];
        for entry in last_third {
            assert_eq!(*entry, MAX_BACKOFF, "tail backoff must saturate at max");
        }
    }

    #[test]
    fn supervisor_stops_when_should_continue_returns_false() {
        let attempts = AtomicU32::new(0);
        let sleeps: Mutex<Vec<Duration>> = Mutex::new(Vec::new());

        let (_tx, rx) = crossbeam_channel::unbounded::<()>();
        let mut runner = |_rx: &Receiver<()>| -> Result<()> {
            attempts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        };
        let mut sleeper = |d: Duration| {
            sleeps.lock().unwrap().push(d);
        };
        let mut trigger = no_op_trigger();
        let should_continue = || false;

        run_supervised(&mut runner, rx, &mut sleeper, &mut trigger, should_continue);

        assert_eq!(attempts.load(Ordering::SeqCst), 0);
        assert!(sleeps.lock().unwrap().is_empty());
    }

    #[test]
    fn supervisor_skips_doctor_trigger_during_transient_retries() {
        let attempts = AtomicU32::new(0);
        let triggers: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let total_attempts = 3;

        let (_tx, rx) = crossbeam_channel::unbounded::<()>();
        let mut runner = |_rx: &Receiver<()>| -> Result<()> {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(anyhow!("transient flap"))
        };
        let mut sleeper = |_d: Duration| {};
        let mut trigger = |reason: &str| {
            triggers.lock().unwrap().push(reason.to_string());
        };
        let should_continue = || attempts.load(Ordering::SeqCst) < total_attempts;

        run_supervised(&mut runner, rx, &mut sleeper, &mut trigger, should_continue);

        assert!(
            triggers.lock().unwrap().is_empty(),
            "doctor trigger must not fire while backoff is below the cap; got {:?}",
            triggers.lock().unwrap()
        );
    }

    #[test]
    fn supervisor_fires_doctor_trigger_once_backoff_saturates() {
        let attempts = AtomicU32::new(0);
        let triggers: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let total_attempts = 12;

        let (_tx, rx) = crossbeam_channel::unbounded::<()>();
        let mut runner = |_rx: &Receiver<()>| -> Result<()> {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(anyhow!("permanent failure"))
        };
        let mut sleeper = |_d: Duration| {};
        let mut trigger = |reason: &str| {
            triggers.lock().unwrap().push(reason.to_string());
        };
        let should_continue = || attempts.load(Ordering::SeqCst) < total_attempts;

        run_supervised(&mut runner, rx, &mut sleeper, &mut trigger, should_continue);

        let fired = triggers.lock().unwrap();
        assert!(
            !fired.is_empty(),
            "doctor trigger must fire after backoff saturates with continued failure"
        );
        for reason in fired.iter() {
            assert!(
                reason.contains("permanent failure"),
                "trigger reason must include the underlying error; got {:?}",
                reason
            );
        }
    }

    #[test]
    fn supervisor_does_not_fire_trigger_on_ok_return_at_saturation() {
        let attempts = AtomicU32::new(0);
        let triggers: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let total_attempts = 12;

        let (_tx, rx) = crossbeam_channel::unbounded::<()>();
        let mut runner = |_rx: &Receiver<()>| -> Result<()> {
            attempts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        };
        let mut sleeper = |_d: Duration| {};
        let mut trigger = |reason: &str| {
            triggers.lock().unwrap().push(reason.to_string());
        };
        let should_continue = || attempts.load(Ordering::SeqCst) < total_attempts;

        run_supervised(&mut runner, rx, &mut sleeper, &mut trigger, should_continue);

        assert!(
            triggers.lock().unwrap().is_empty(),
            "Ok return is not a doctor-worthy event even at saturation"
        );
    }
}
