use std::collections::HashSet;
use std::time::{Duration, Instant};

use anyhow::Result;

use super::compile::{self, Compiled};
use super::document::Synced;
use super::service::{self, Probe};
use super::store::Store;
use crate::platform::engine::config::KeyBackendChoice;
use crate::platform::engine::remap::ResolvedConfig;
use crate::platform::macos::input::selection::Availability;
use crate::platform::macos::input::{BackendId, InputState};
use crate::platform::macos::layout::LayoutSnapshot;

const PROBE_INTERVAL: Duration = Duration::from_secs(2);

pub(crate) trait Karabiner {
    fn probe(&self) -> Probe;
    fn sync(&self, compiled: &Compiled) -> Result<Synced>;
    fn remove(&self) -> Result<()>;
    fn set_active(&self, active: bool) -> Result<()>;
}

pub(crate) struct Installed(pub(crate) Store);

impl Karabiner for Installed {
    fn probe(&self) -> Probe {
        service::probe(&self.0)
    }

    fn sync(&self, compiled: &Compiled) -> Result<Synced> {
        self.0.sync(compiled)
    }

    fn remove(&self) -> Result<()> {
        self.0.remove().map(|_| ())
    }

    fn set_active(&self, active: bool) -> Result<()> {
        service::set_active(active)
    }
}

#[derive(Debug, PartialEq, Eq)]
enum State {
    Unknown,
    Off,
    Waiting {
        next_probe: Instant,
    },
    Active {
        core_pid: Option<u32>,
        layout: String,
        enabled: bool,
        next_probe: Instant,
    },
}

pub(crate) struct Backend<K> {
    karabiner: K,
    state: State,
    said: HashSet<String>,
}

impl<K: Karabiner> Backend<K> {
    pub(crate) fn new(karabiner: K) -> Self {
        Self {
            karabiner,
            state: State::Unknown,
            said: HashSet::new(),
        }
    }

    pub(crate) fn apply(
        &mut self,
        choice: KeyBackendChoice,
        config: &ResolvedConfig,
        layout: &LayoutSnapshot,
        input: &InputState,
    ) {
        if choice != KeyBackendChoice::KarabinerElements {
            self.release(input);
            return;
        }
        match self.state {
            State::Active { .. } => self.refresh(config, layout),
            _ => self.try_activate(config, layout, input, Instant::now()),
        }
    }

    pub(crate) fn idle(
        &mut self,
        config: &ResolvedConfig,
        layout: &LayoutSnapshot,
        input: &InputState,
    ) {
        self.idle_at(config, layout, input, Instant::now());
    }

    fn idle_at(
        &mut self,
        config: &ResolvedConfig,
        layout: &LayoutSnapshot,
        input: &InputState,
        now: Instant,
    ) {
        let (stale, due) = match &self.state {
            State::Waiting { next_probe } => (false, now >= *next_probe),
            State::Active {
                layout: compiled_for,
                next_probe,
                enabled,
                ..
            } => (
                *compiled_for != layout.id || *enabled != config.enabled,
                now >= *next_probe,
            ),
            State::Unknown | State::Off => return,
        };
        if matches!(self.state, State::Waiting { .. }) {
            if due {
                self.try_activate(config, layout, input, now);
            }
            return;
        }
        if stale {
            self.refresh(config, layout);
        }
        if due {
            self.check_service(config, input, now);
        }
    }

    pub(crate) fn stop(&mut self) {
        match self.state {
            State::Active { .. } => self.report("turning off", self.karabiner.set_active(false)),
            State::Waiting { .. } => self.turn_off_if_running(self.karabiner.probe().core_pid),
            State::Unknown | State::Off => {}
        }
        self.state = State::Off;
    }

    fn turn_off_if_running(&self, core_pid: Option<u32>) {
        if core_pid.is_none() {
            return;
        }
        if let Err(error) = self.karabiner.set_active(false) {
            log::debug!("could not turn the Key Remap variable off: {error:#}");
        }
    }

    fn try_activate(
        &mut self,
        config: &ResolvedConfig,
        layout: &LayoutSnapshot,
        input: &InputState,
        now: Instant,
    ) {
        let probe = self.karabiner.probe();
        if let Availability::Missing { what, fix } = &probe.availability {
            self.say(format!(
                "Karabiner-Elements is chosen but needs {what}, so the event tap remaps keys: {fix}"
            ));
            self.state = State::Waiting {
                next_probe: now + PROBE_INTERVAL,
            };
            return;
        }
        let compiled = compile::compile(config, layout);
        if !self.write(&compiled) {
            self.state = State::Waiting {
                next_probe: now + PROBE_INTERVAL,
            };
            return;
        }
        input.backend.set(BackendId::KarabinerElements);
        if let Err(error) = self.karabiner.set_active(config.enabled) {
            input.backend.set(BackendId::EventTap);
            self.say(format!(
                "could not set the Key Remap variable in Karabiner-Elements, so the event tap remaps keys: {error:#}"
            ));
            self.state = State::Waiting {
                next_probe: now + PROBE_INTERVAL,
            };
            return;
        }
        log::info!("key backend: karabiner_elements");
        self.said.clear();
        self.state = State::Active {
            core_pid: probe.core_pid,
            layout: layout.id.clone(),
            enabled: config.enabled,
            next_probe: now + PROBE_INTERVAL,
        };
    }

    fn refresh(&mut self, config: &ResolvedConfig, layout: &LayoutSnapshot) {
        let compiled = compile::compile(config, layout);
        let written = self.write(&compiled);
        let result = self.karabiner.set_active(config.enabled);
        let set = result.is_ok();
        self.report("updating", result);
        if let State::Active {
            layout: compiled_for,
            enabled,
            ..
        } = &mut self.state
        {
            if written {
                *compiled_for = layout.id.clone();
            }
            if set {
                *enabled = config.enabled;
            }
        }
    }

    fn check_service(&mut self, config: &ResolvedConfig, input: &InputState, now: Instant) {
        let probe = self.karabiner.probe();
        if let Availability::Missing { what, fix } = probe.availability {
            self.turn_off_if_running(probe.core_pid);
            input.backend.set(BackendId::EventTap);
            log::warn!("Karabiner-Elements needs {what}, so the event tap remaps keys until it is back: {fix}");
            self.state = State::Waiting {
                next_probe: now + PROBE_INTERVAL,
            };
            return;
        }
        let State::Active {
            core_pid,
            enabled,
            next_probe,
            ..
        } = &mut self.state
        else {
            return;
        };
        *next_probe = now + PROBE_INTERVAL;
        if probe.core_pid != *core_pid {
            *core_pid = probe.core_pid;
            log::info!("Karabiner-Elements restarted; setting the Key Remap variable again");
            match self.karabiner.set_active(config.enabled) {
                Ok(()) => *enabled = config.enabled,
                Err(error) => {
                    *enabled = !config.enabled;
                    self.report("turning on", Err(error));
                }
            }
        }
    }

    fn release(&mut self, input: &InputState) {
        match self.state {
            State::Off => return,
            State::Active { .. } => {
                self.report("turning off", self.karabiner.set_active(false));
                input.backend.set(BackendId::EventTap);
                self.report("removing the rules", self.karabiner.remove());
                log::info!("key backend: left karabiner_elements");
            }
            State::Unknown | State::Waiting { .. } => {
                self.turn_off_if_running(self.karabiner.probe().core_pid);
                if let Err(error) = self.karabiner.remove() {
                    log::debug!("no Karabiner-Elements rules to clean up: {error:#}");
                }
            }
        }
        self.state = State::Off;
    }

    fn write(&mut self, compiled: &Compiled) -> bool {
        match self.karabiner.sync(compiled) {
            Ok(synced) => {
                if synced.edited_by_hand {
                    log::warn!("the Key Remap block in karabiner.json had been edited in Karabiner-Elements; it was replaced");
                }
                for key in &synced.conflicts {
                    self.say(format!(
                        "karabiner.json already changes {key}, so Key Remap's modifier key change for {key} is not applied"
                    ));
                }
                for gap in &compiled.gaps {
                    self.say(format!(
                        "not compiled for Karabiner-Elements: {}: {}",
                        gap.rule, gap.reason
                    ));
                }
                true
            }
            Err(error) => {
                self.say(format!(
                    "could not write the Key Remap rules into karabiner.json: {error:#}"
                ));
                false
            }
        }
    }

    fn report(&mut self, doing: &str, result: Result<()>) {
        if let Err(error) = result {
            self.say(format!(
                "Karabiner-Elements: {doing} the Key Remap variable failed: {error:#}"
            ));
        }
    }

    fn say(&mut self, message: String) {
        if self.said.insert(message.clone()) {
            log::warn!("{message}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use qol_hotkeys::macos_keycode::PhysicalLayout;
    use serde_json::json;

    use super::*;
    use crate::platform::engine::config::RemapConfig;
    use crate::platform::engine::remap;
    use crate::platform::macos::layout::CharTable;

    #[derive(Clone, Default)]
    struct Fake {
        calls: Rc<RefCell<Vec<String>>>,
        probe: Rc<RefCell<Option<Probe>>>,
        cli_fails: Rc<std::cell::Cell<bool>>,
    }

    impl Fake {
        fn ready(&self, pid: u32) {
            *self.probe.borrow_mut() = Some(Probe {
                availability: Availability::Ready,
                core_pid: Some(pid),
            });
        }

        fn missing(&self) {
            self.missing_with(None);
        }

        fn missing_with(&self, core_pid: Option<u32>) {
            *self.probe.borrow_mut() = Some(Probe {
                availability: Availability::Missing {
                    what: "Karabiner-Elements running".to_string(),
                    fix: "open it".to_string(),
                },
                core_pid,
            });
        }

        fn take(&self) -> Vec<String> {
            std::mem::take(&mut *self.calls.borrow_mut())
        }
    }

    impl Karabiner for Fake {
        fn probe(&self) -> Probe {
            self.probe.borrow().clone().expect("set a probe first")
        }

        fn sync(&self, compiled: &Compiled) -> Result<Synced> {
            let count = compiled.rule["manipulators"].as_array().map_or(0, Vec::len);
            self.calls.borrow_mut().push(format!("sync {count}"));
            Ok(Synced {
                document: json!({}),
                sidecar: Default::default(),
                profile: "Default profile".to_string(),
                changed: true,
                edited_by_hand: false,
                conflicts: Vec::new(),
            })
        }

        fn remove(&self) -> Result<()> {
            self.calls.borrow_mut().push("remove".to_string());
            Ok(())
        }

        fn set_active(&self, active: bool) -> Result<()> {
            self.calls.borrow_mut().push(format!("active {active}"));
            if self.cli_fails.get() {
                anyhow::bail!("karabiner_cli failed");
            }
            Ok(())
        }
    }

    fn config(enabled: bool) -> ResolvedConfig {
        let raw: RemapConfig = serde_json::from_value(json!({
            "enabled": enabled,
            "key_rules": [{ "from_mods": ["ctrl"], "from_key": "c", "to_mods": ["cmd"], "to_key": "c" }]
        }))
        .unwrap();
        remap::resolve(&raw)
    }

    fn layout(id: &str) -> LayoutSnapshot {
        LayoutSnapshot {
            id: id.to_string(),
            table: CharTable::from_translation(|_, _, _| None),
            physical: PhysicalLayout::Ansi,
        }
    }

    const CHOSEN: KeyBackendChoice = KeyBackendChoice::KarabinerElements;

    #[test]
    fn choosing_karabiner_writes_the_rules_then_owns_the_keys_then_turns_them_on() {
        let fake = Fake::default();
        fake.ready(10);
        let input = InputState::default();
        let mut backend = Backend::new(fake.clone());

        backend.apply(CHOSEN, &config(true), &layout("us"), &input);

        assert_eq!(fake.take(), vec!["sync 1", "active true"]);
        assert_eq!(input.backend.get(), BackendId::KarabinerElements);
    }

    #[test]
    fn toggling_off_keeps_the_rules_and_turns_the_variable_off() {
        let fake = Fake::default();
        fake.ready(10);
        let input = InputState::default();
        let mut backend = Backend::new(fake.clone());
        backend.apply(CHOSEN, &config(true), &layout("us"), &input);
        fake.take();

        backend.apply(CHOSEN, &config(false), &layout("us"), &input);

        assert_eq!(fake.take(), vec!["sync 1", "active false"]);
        assert_eq!(input.backend.get(), BackendId::KarabinerElements);
    }

    #[test]
    fn switching_away_turns_off_before_handing_keys_back_then_removes() {
        let fake = Fake::default();
        fake.ready(10);
        let input = InputState::default();
        let mut backend = Backend::new(fake.clone());
        backend.apply(CHOSEN, &config(true), &layout("us"), &input);
        fake.take();

        backend.apply(
            KeyBackendChoice::EventTap,
            &config(true),
            &layout("us"),
            &input,
        );

        assert_eq!(fake.take(), vec!["active false", "remove"]);
        assert_eq!(input.backend.get(), BackendId::EventTap);
        backend.apply(
            KeyBackendChoice::EventTap,
            &config(true),
            &layout("us"),
            &input,
        );
        assert!(fake.take().is_empty());
    }

    #[test]
    fn a_daemon_started_on_another_backend_cleans_up_once() {
        let fake = Fake::default();
        fake.ready(10);
        let input = InputState::default();
        let mut backend = Backend::new(fake.clone());

        backend.apply(KeyBackendChoice::Auto, &config(true), &layout("us"), &input);
        backend.apply(KeyBackendChoice::Auto, &config(true), &layout("us"), &input);

        assert_eq!(fake.take(), vec!["active false", "remove"]);
        assert_eq!(input.backend.get(), BackendId::EventTap);
    }

    #[test]
    fn a_missing_karabiner_leaves_keys_on_the_tap_and_retries() {
        let fake = Fake::default();
        fake.missing();
        let input = InputState::default();
        let mut backend = Backend::new(fake.clone());
        let start = Instant::now();

        backend.apply(CHOSEN, &config(true), &layout("us"), &input);
        assert!(fake.take().is_empty());
        assert_eq!(input.backend.get(), BackendId::EventTap);

        fake.ready(10);
        backend.idle_at(&config(true), &layout("us"), &input, start);
        assert!(fake.take().is_empty(), "waits for the retry interval");
        backend.idle_at(
            &config(true),
            &layout("us"),
            &input,
            start + PROBE_INTERVAL * 2,
        );
        assert_eq!(fake.take(), vec!["sync 1", "active true"]);
        assert_eq!(input.backend.get(), BackendId::KarabinerElements);
    }

    #[test]
    fn a_core_service_restart_sets_the_variable_again() {
        let fake = Fake::default();
        fake.ready(10);
        let input = InputState::default();
        let mut backend = Backend::new(fake.clone());
        let start = Instant::now();
        backend.apply(CHOSEN, &config(true), &layout("us"), &input);
        fake.take();

        fake.ready(11);
        backend.idle_at(
            &config(true),
            &layout("us"),
            &input,
            start + PROBE_INTERVAL * 2,
        );
        assert_eq!(fake.take(), vec!["active true"]);
        backend.idle_at(
            &config(true),
            &layout("us"),
            &input,
            start + PROBE_INTERVAL * 4,
        );
        assert!(fake.take().is_empty());
    }

    #[test]
    fn karabiner_going_away_hands_keys_back_to_the_tap() {
        let fake = Fake::default();
        fake.ready(10);
        let input = InputState::default();
        let mut backend = Backend::new(fake.clone());
        let start = Instant::now();
        backend.apply(CHOSEN, &config(true), &layout("us"), &input);
        fake.take();

        fake.missing();
        backend.idle_at(
            &config(true),
            &layout("us"),
            &input,
            start + PROBE_INTERVAL * 2,
        );
        assert_eq!(input.backend.get(), BackendId::EventTap);
        assert!(fake.take().is_empty());
    }

    #[test]
    fn a_layout_change_recompiles() {
        let fake = Fake::default();
        fake.ready(10);
        let input = InputState::default();
        let mut backend = Backend::new(fake.clone());
        let start = Instant::now();
        backend.apply(CHOSEN, &config(true), &layout("us"), &input);
        fake.take();

        backend.idle_at(&config(true), &layout("danish"), &input, start);
        assert_eq!(fake.take(), vec!["sync 1", "active true"]);
        backend.idle_at(&config(true), &layout("danish"), &input, start);
        assert!(fake.take().is_empty());
    }

    #[test]
    fn stopping_the_daemon_turns_the_variable_off() {
        let fake = Fake::default();
        fake.ready(10);
        let input = InputState::default();
        let mut backend = Backend::new(fake.clone());
        backend.apply(CHOSEN, &config(true), &layout("us"), &input);
        fake.take();

        backend.stop();
        assert_eq!(fake.take(), vec!["active false"]);
    }

    #[test]
    fn karabiner_still_running_but_not_ready_turns_the_variable_off_on_fallback() {
        let fake = Fake::default();
        fake.ready(10);
        let input = InputState::default();
        let mut backend = Backend::new(fake.clone());
        let start = Instant::now();
        backend.apply(CHOSEN, &config(true), &layout("us"), &input);
        fake.take();

        fake.missing_with(Some(10));
        backend.idle_at(
            &config(true),
            &layout("us"),
            &input,
            start + PROBE_INTERVAL * 2,
        );
        assert_eq!(fake.take(), vec!["active false"]);
        assert_eq!(input.backend.get(), BackendId::EventTap);

        backend.apply(
            KeyBackendChoice::EventTap,
            &config(true),
            &layout("us"),
            &input,
        );
        assert_eq!(fake.take(), vec!["active false", "remove"]);
    }

    #[test]
    fn a_failed_variable_keeps_keys_on_the_tap_and_retries() {
        let fake = Fake::default();
        fake.ready(10);
        fake.cli_fails.set(true);
        let input = InputState::default();
        let mut backend = Backend::new(fake.clone());
        let start = Instant::now();

        backend.apply(CHOSEN, &config(true), &layout("us"), &input);
        assert_eq!(fake.take(), vec!["sync 1", "active true"]);
        assert_eq!(input.backend.get(), BackendId::EventTap);

        fake.cli_fails.set(false);
        backend.idle_at(
            &config(true),
            &layout("us"),
            &input,
            start + PROBE_INTERVAL * 2,
        );
        assert_eq!(fake.take(), vec!["sync 1", "active true"]);
        assert_eq!(input.backend.get(), BackendId::KarabinerElements);
    }

    #[test]
    fn a_failed_toggle_is_retried_on_the_next_idle() {
        let fake = Fake::default();
        fake.ready(10);
        let input = InputState::default();
        let mut backend = Backend::new(fake.clone());
        let start = Instant::now();
        backend.apply(CHOSEN, &config(true), &layout("us"), &input);
        fake.take();

        fake.cli_fails.set(true);
        backend.apply(CHOSEN, &config(false), &layout("us"), &input);
        fake.take();
        fake.cli_fails.set(false);
        backend.idle_at(&config(false), &layout("us"), &input, start);
        assert_eq!(fake.take(), vec!["sync 1", "active false"]);
        backend.idle_at(&config(false), &layout("us"), &input, start);
        assert!(fake.take().is_empty());
    }
}
