use super::catalog::load_available_actions;
use super::reload;
use super::{HotkeyAction, HotkeyManager};
use crate::plugins::PluginManager;
use anyhow::{anyhow, Result};
use crossbeam_channel::{after, never, select, Receiver};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyEventReceiver, HotKeyState};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const INITIAL_BACKOFF: Duration = Duration::from_millis(500);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(100);

type SharedPluginManager = Arc<Mutex<PluginManager>>;

pub fn start_hotkey_listener(plugin_manager: Arc<Mutex<PluginManager>>) -> Result<()> {
    let reload_rx = reload::subscribe();
    std::thread::spawn(move || {
        run_supervised(
            &mut |reload_rx| run_listener_once(&plugin_manager, reload_rx),
            reload_rx,
            &mut std::thread::sleep,
            &mut mark_doctor_needed,
            || true,
        );
    });
    Ok(())
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

fn run_listener_once(plugin_manager: &SharedPluginManager, reload_rx: &Receiver<()>) -> Result<()> {
    let manager =
        HotkeyManager::new().map_err(|e| anyhow!("failed to create hotkey manager: {}", e))?;
    HotkeyListenerLoop {
        manager,
        plugin_manager: plugin_manager.clone(),
        reload_rx,
        held_actions: HeldActions::default(),
    }
    .run();
    Err(anyhow!("hotkey listener loop returned unexpectedly"))
}

struct HotkeyListenerLoop<'a> {
    manager: HotkeyManager,
    plugin_manager: SharedPluginManager,
    reload_rx: &'a Receiver<()>,
    held_actions: HeldActions,
}

impl<'a> HotkeyListenerLoop<'a> {
    fn register_initial_hotkeys(&mut self) {
        if let Err(error) = self.reload_hotkeys() {
            log::error!("Failed to register hotkeys: {}", error);
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

        loop {
            let heartbeat_rx: Receiver<Instant> = if self.held_actions.is_empty() {
                never()
            } else {
                after(HEARTBEAT_INTERVAL)
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
                recv(heartbeat_rx) -> _ => self.send_heartbeats(),
            }
        }
        self.stop_held_actions();
    }

    fn drain_reload_signals(&self) {
        while self.reload_rx.try_recv().is_ok() {}
    }

    fn handle_reload(&mut self) {
        log::info!("Reloading hotkeys...");
        self.stop_held_actions();
        match self.reload_hotkeys() {
            Ok(()) => log::info!("Hotkeys reloaded successfully"),
            Err(error) => log::error!("Failed to register hotkeys: {}", error),
        }
    }

    fn handle_hotkey_event(&mut self, event: GlobalHotKeyEvent) {
        let action = self.manager.get_action(&event).cloned();
        let Some(dispatch) = self.held_actions.handle_event(event, action.as_ref()) else {
            return;
        };
        self.dispatch(dispatch);
    }

    fn send_heartbeats(&self) {
        for action in self.held_actions.heartbeat_actions() {
            self.dispatch(HotkeyDispatch::Continuous {
                action,
                phase: ContinuousPhase::Heartbeat,
            });
        }
    }

    fn stop_held_actions(&mut self) {
        for dispatch in self.held_actions.stop_all() {
            self.dispatch(dispatch);
        }
    }

    fn dispatch(&self, dispatch: HotkeyDispatch) {
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
                crate::plugins::action_executor::execute_action(
                    &self.plugin_manager,
                    &plugin_id,
                    &action.action,
                );
            }
            HotkeyDispatch::Continuous { action, phase } => {
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
                    serde_json::json!({ "phase": phase.as_str() }),
                );
            }
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
    Continuous {
        action: HotkeyAction,
        phase: ContinuousPhase,
    },
}

impl HotkeyDispatch {
    fn action(&self) -> &HotkeyAction {
        match self {
            Self::OneShot(action) | Self::Continuous { action, .. } => action,
        }
    }
}

#[derive(Default)]
struct HeldActions {
    actions: HashMap<u32, HotkeyAction>,
}

impl HeldActions {
    fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    fn handle_event(
        &mut self,
        event: GlobalHotKeyEvent,
        registered_action: Option<&HotkeyAction>,
    ) -> Option<HotkeyDispatch> {
        match event.state {
            HotKeyState::Pressed => {
                let action = registered_action?.clone();
                if !action.continuous {
                    return Some(HotkeyDispatch::OneShot(action));
                }
                if self.actions.contains_key(&event.id) {
                    return None;
                }
                self.actions.insert(event.id, action.clone());
                Some(HotkeyDispatch::Continuous {
                    action,
                    phase: ContinuousPhase::Start,
                })
            }
            HotKeyState::Released => {
                let action = self.actions.remove(&event.id)?;
                Some(HotkeyDispatch::Continuous {
                    action,
                    phase: ContinuousPhase::Stop,
                })
            }
        }
    }

    fn heartbeat_actions(&self) -> Vec<HotkeyAction> {
        self.actions.values().cloned().collect()
    }

    fn stop_all(&mut self) -> Vec<HotkeyDispatch> {
        self.actions
            .drain()
            .map(|(_, action)| HotkeyDispatch::Continuous {
                action,
                phase: ContinuousPhase::Stop,
            })
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::PluginUid;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn action(name: &str, continuous: bool) -> HotkeyAction {
        HotkeyAction {
            plugin_uid: PluginUid::new("uid-window-actions"),
            action: name.to_string(),
            continuous,
        }
    }

    fn event(id: u32, state: HotKeyState) -> GlobalHotKeyEvent {
        GlobalHotKeyEvent { id, state }
    }

    #[test]
    fn one_shot_actions_dispatch_only_on_press() {
        let mut held = HeldActions::default();
        let center = action("center", false);

        assert_eq!(
            held.handle_event(event(1, HotKeyState::Pressed), Some(&center)),
            Some(HotkeyDispatch::OneShot(center))
        );
        assert_eq!(
            held.handle_event(event(1, HotKeyState::Released), None),
            None
        );
        assert!(held.is_empty());
    }

    #[test]
    fn continuous_actions_dispatch_start_heartbeats_and_stop() {
        let mut held = HeldActions::default();
        let glide = action("glide-left", true);

        assert_eq!(
            held.handle_event(event(7, HotKeyState::Pressed), Some(&glide)),
            Some(HotkeyDispatch::Continuous {
                action: glide.clone(),
                phase: ContinuousPhase::Start,
            })
        );
        assert_eq!(held.heartbeat_actions(), vec![glide.clone()]);
        assert_eq!(
            held.handle_event(event(7, HotKeyState::Released), Some(&glide)),
            Some(HotkeyDispatch::Continuous {
                action: glide,
                phase: ContinuousPhase::Stop,
            })
        );
        assert!(held.is_empty());
    }

    #[test]
    fn continuous_actions_ignore_duplicate_press_events() {
        let mut held = HeldActions::default();
        let glide = action("glide-right", true);

        assert!(held
            .handle_event(event(9, HotKeyState::Pressed), Some(&glide))
            .is_some());
        assert_eq!(
            held.handle_event(event(9, HotKeyState::Pressed), Some(&glide)),
            None
        );
        assert_eq!(held.heartbeat_actions(), vec![glide]);
    }

    #[test]
    fn stop_all_releases_every_held_continuous_action() {
        let mut held = HeldActions::default();
        let left = action("glide-left", true);
        let up = action("glide-up", true);
        held.handle_event(event(1, HotKeyState::Pressed), Some(&left));
        held.handle_event(event(2, HotKeyState::Pressed), Some(&up));

        let mut stopped = held.stop_all();
        stopped.sort_by(|a, b| a.action().action.cmp(&b.action().action));

        assert_eq!(
            stopped,
            vec![
                HotkeyDispatch::Continuous {
                    action: left,
                    phase: ContinuousPhase::Stop,
                },
                HotkeyDispatch::Continuous {
                    action: up,
                    phase: ContinuousPhase::Stop,
                },
            ]
        );
        assert!(held.is_empty());
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
