use super::catalog::load_available_actions;
use super::HotkeyManager;
use crate::plugins::PluginManager;
use anyhow::{anyhow, Result};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyEventReceiver, HotKeyState};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

static RELOAD_SENDER: OnceLock<Sender<()>> = OnceLock::new();
const HOTKEY_LOOP_SLEEP_MS: u64 = 10;
const INITIAL_BACKOFF: Duration = Duration::from_millis(500);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

type SharedPluginManager = Arc<Mutex<PluginManager>>;

pub fn trigger_reload() {
    if let Some(sender) = RELOAD_SENDER.get() {
        let _ = sender.send(());
    }
}

pub fn start_hotkey_listener(plugin_manager: Arc<Mutex<PluginManager>>) -> Result<()> {
    let reload_rx = install_reload_channel();
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
    }
    .run();
    Err(anyhow!("hotkey listener loop returned unexpectedly"))
}

struct HotkeyListenerLoop<'a> {
    manager: HotkeyManager,
    plugin_manager: SharedPluginManager,
    reload_rx: &'a Receiver<()>,
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
        let hotkey_receiver = GlobalHotKeyEvent::receiver();

        loop {
            self.try_reload_hotkeys();
            self.try_handle_hotkeys(hotkey_receiver);
            sleep_between_polls();
        }
    }

    fn try_handle_hotkeys(&self, receiver: &GlobalHotKeyEventReceiver) {
        while let Ok(event) = receiver.try_recv() {
            self.handle_hotkey_event(event);
        }
    }

    fn try_reload_hotkeys(&mut self) {
        if !reload_requested(self.reload_rx) {
            return;
        }

        log::info!("Reloading hotkeys...");

        match self.reload_hotkeys() {
            Ok(()) => log::info!("Hotkeys reloaded successfully"),
            Err(error) => log::error!("Failed to register hotkeys: {}", error),
        }
    }

    fn handle_hotkey_event(&self, event: GlobalHotKeyEvent) {
        if event.state != HotKeyState::Pressed {
            return;
        }

        let Some(action) = self.manager.get_action(&event) else {
            return;
        };

        log::info!("Hotkey triggered: {}::{}", action.plugin_id, action.action);

        crate::plugins::action_executor::execute_action(
            &self.plugin_manager,
            &action.plugin_id,
            &action.action,
        );
    }
}
fn install_reload_channel() -> Receiver<()> {
    let (reload_tx, reload_rx) = mpsc::channel::<()>();
    let _ = RELOAD_SENDER.set(reload_tx);
    reload_rx
}

fn reload_requested(reload_rx: &Receiver<()>) -> bool {
    let mut requested = false;

    while reload_rx.try_recv().is_ok() {
        requested = true;
    }

    requested
}

fn sleep_between_polls() {
    std::thread::sleep(std::time::Duration::from_millis(HOTKEY_LOOP_SLEEP_MS));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

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

        let (_tx, rx) = mpsc::channel::<()>();
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

        let (_tx, rx) = mpsc::channel::<()>();
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

        let (_tx, rx) = mpsc::channel::<()>();
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

        let (_tx, rx) = mpsc::channel::<()>();
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

        let (_tx, rx) = mpsc::channel::<()>();
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

        let (_tx, rx) = mpsc::channel::<()>();
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

        let (_tx, rx) = mpsc::channel::<()>();
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
