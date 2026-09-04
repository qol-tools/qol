use super::super::super::binding::{Binding, CaptureEvent, Phase};
use super::evdev_backend::keycode_name;
use qol_hotkeys::evdev;
use qol_hotkeys::grammar::Modifier;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};

const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CaptureDecision {
    pub(super) forward: bool,
    pub(super) events: Vec<CaptureEvent>,
}

impl CaptureDecision {
    fn forward() -> Self {
        Self {
            forward: true,
            events: Vec::new(),
        }
    }

    fn suppress(events: Vec<CaptureEvent>) -> Self {
        Self {
            forward: false,
            events,
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct BindingMatcher {
    bindings: Vec<(LinuxCombo, Binding)>,
    state: evdev::ModifierState,
    #[cfg(debug_assertions)]
    traced_modifiers: std::collections::BTreeSet<Modifier>,
    active_continuous: HashMap<u16, (LinuxCombo, Binding, Instant)>,
    suppressed_keys: HashSet<u16>,
    held: HashSet<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LinuxCombo {
    mods: BTreeSet<Modifier>,
    key: u16,
}

impl BindingMatcher {
    pub(super) fn new(bindings: Vec<Binding>) -> Self {
        let bindings = linux_bindings(bindings);
        Self {
            bindings,
            ..Self::default()
        }
    }

    pub(super) fn observe(&mut self, code: u16, value: i32) -> CaptureDecision {
        if self.state.handle(code, value) {
            let decision = if value == 1 && !self.held.insert(code) {
                CaptureDecision::suppress(Vec::new())
            } else {
                if value == 0 {
                    self.held.remove(&code);
                }
                self.modifier_decision()
            };
            #[cfg(debug_assertions)]
            {
                let mods = self.state.pressed_modifiers();
                if mods != self.traced_modifiers {
                    self.traced_modifiers = mods.clone();
                    qol_runtime::probe!(
                        "HOTKEY_CAPTURE",
                        "event=mods code={} value={} mods={}",
                        keycode_name(code),
                        value,
                        mods.iter()
                            .map(|m| format!("{m:?}"))
                            .collect::<Vec<_>>()
                            .join(",")
                    );
                }
            }
            return decision;
        }
        match value {
            0 => {
                self.held.remove(&code);
                self.release_key(code)
            }
            1 => {
                if !self.held.insert(code) {
                    return CaptureDecision::suppress(Vec::new());
                }
                self.press_key(code)
            }
            2 if self.suppressed_keys.contains(&code) => self.repeat_key(code),
            _ => CaptureDecision::forward(),
        }
    }

    pub(super) fn seed_held(&mut self, codes: impl IntoIterator<Item = u16>) {
        for code in codes {
            self.held.insert(code);
            self.state.handle(code, 1);
        }
    }

    pub(super) fn reconcile(&mut self, code: u16, value: i32) -> Vec<CaptureEvent> {
        self.state.handle(code, value);
        if value != 0 {
            self.held.insert(code);
            return Vec::new();
        }
        self.held.remove(&code);
        self.release_key(code).events
    }

    pub(super) fn reload(&mut self, bindings: Vec<Binding>) -> Vec<CaptureEvent> {
        self.bindings = linux_bindings(bindings);
        self.active_continuous
            .drain()
            .map(|(_, (_, binding, _))| CaptureEvent {
                binding,
                phase: Phase::STOP,
            })
            .collect()
    }

    fn modifier_decision(&mut self) -> CaptureDecision {
        let modifiers = self.state.pressed_modifiers();
        let ended: Vec<u16> = self
            .active_continuous
            .iter()
            .filter_map(|(code, (combo, _, _))| (combo.mods != modifiers).then_some(*code))
            .collect();
        let events = ended
            .into_iter()
            .filter_map(|code| self.active_continuous.remove(&code))
            .map(|(_, binding, _)| CaptureEvent {
                binding,
                phase: Phase::STOP,
            })
            .collect();
        CaptureDecision {
            forward: true,
            events,
        }
    }

    fn press_key(&mut self, code: u16) -> CaptureDecision {
        let modifiers = self.state.pressed_modifiers();
        let Some((combo, binding)) = self
            .bindings
            .iter()
            .find(|(combo, _)| combo.key == code && combo.mods == modifiers)
            .cloned()
        else {
            return CaptureDecision::forward();
        };
        self.suppressed_keys.insert(code);
        if binding.continuous {
            self.active_continuous
                .insert(code, (combo, binding.clone(), Instant::now()));
        }
        CaptureDecision::suppress(vec![CaptureEvent {
            binding,
            phase: Phase::START,
        }])
    }

    fn repeat_key(&mut self, code: u16) -> CaptureDecision {
        let event =
            self.active_continuous
                .get_mut(&code)
                .and_then(|(_, binding, last_heartbeat)| {
                    if last_heartbeat.elapsed() < HEARTBEAT_INTERVAL {
                        return None;
                    }
                    *last_heartbeat = Instant::now();
                    Some(CaptureEvent {
                        binding: binding.clone(),
                        phase: Phase::HEARTBEAT,
                    })
                });
        CaptureDecision::suppress(event.into_iter().collect())
    }

    fn release_key(&mut self, code: u16) -> CaptureDecision {
        let suppressed = self.suppressed_keys.remove(&code);
        let event = self
            .active_continuous
            .remove(&code)
            .map(|(_, binding, _)| CaptureEvent {
                binding,
                phase: Phase::STOP,
            });
        if !suppressed {
            return CaptureDecision::forward();
        }
        CaptureDecision::suppress(event.into_iter().collect())
    }
}

fn linux_bindings(bindings: Vec<Binding>) -> Vec<(LinuxCombo, Binding)> {
    bindings
        .into_iter()
        .filter_map(|binding| {
            let combo = binding.combo.as_ref()?;
            let key = crate::hotkeys::platform::evdev_keycode(combo.key)?;
            Some((
                LinuxCombo {
                    mods: combo.mods.clone(),
                    key,
                },
                binding,
            ))
        })
        .collect()
}

#[cfg(test)]
mod matcher_tests {
    use super::*;
    use crate::hotkeys::capture::parse_combo;
    use qol_hotkeys::evdev as keycodes;
    use qol_hotkeys::grammar::Modifier;

    fn binding(key: &str, continuous: bool) -> Binding {
        Binding {
            combo: parse_combo(key),
            plugin_uid: crate::plugins::PluginUid::new("plugin"),
            action: "action".into(),
            raw_key: key.into(),
            continuous,
        }
    }

    #[test]
    fn regular_binding_fires_once_and_suppresses_its_key_stream() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Space", false)]);
        assert!(matcher.observe(keycodes::KEY_LEFTMETA, 1).forward);

        let press = matcher.observe(keycodes::KEY_SPACE, 1);
        let repeat = matcher.observe(keycodes::KEY_SPACE, 2);
        let release = matcher.observe(keycodes::KEY_SPACE, 0);

        assert!(!press.forward);
        assert_eq!(press.events[0].phase, Phase::START);
        assert!(!repeat.forward);
        assert!(repeat.events.is_empty());
        assert!(!release.forward);
        assert!(release.events.is_empty());
    }

    #[test]
    fn continuous_binding_emits_start_heartbeat_and_stop() {
        let mut matcher = BindingMatcher::new(vec![binding("Ctrl+Alt+Shift+Left", true)]);
        matcher.observe(keycodes::KEY_LEFTCTRL, 1);
        matcher.observe(keycodes::KEY_LEFTALT, 1);
        matcher.observe(keycodes::KEY_LEFTSHIFT, 1);

        let start = matcher.observe(keycodes::KEY_LEFT, 1);
        if let Some((_, _, last_heartbeat)) = matcher.active_continuous.get_mut(&keycodes::KEY_LEFT)
        {
            *last_heartbeat = Instant::now() - HEARTBEAT_INTERVAL;
        }
        let repeat = matcher.observe(keycodes::KEY_LEFT, 2);
        let stop = matcher.observe(keycodes::KEY_LEFT, 0);

        assert_eq!(start.events[0].phase, Phase::START);
        assert_eq!(repeat.events[0].phase, Phase::HEARTBEAT);
        assert_eq!(stop.events[0].phase, Phase::STOP);
        assert!(!start.forward && !repeat.forward && !stop.forward);
    }

    #[test]
    fn modifier_release_stops_active_continuous_binding() {
        let mut matcher = BindingMatcher::new(vec![binding("Ctrl+Alt+Shift+Right", true)]);
        matcher.observe(keycodes::KEY_LEFTCTRL, 1);
        matcher.observe(keycodes::KEY_LEFTALT, 1);
        matcher.observe(keycodes::KEY_LEFTSHIFT, 1);
        matcher.observe(keycodes::KEY_RIGHT, 1);

        let stop = matcher.observe(keycodes::KEY_LEFTSHIFT, 0);
        let key_release = matcher.observe(keycodes::KEY_RIGHT, 0);

        assert!(stop.forward);
        assert_eq!(stop.events[0].phase, Phase::STOP);
        assert!(!key_release.forward);
        assert!(key_release.events.is_empty());
    }

    #[test]
    fn unmatched_key_with_modifiers_forwards() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Space", false)]);
        matcher.observe(keycodes::KEY_LEFTMETA, 1);
        assert!(matcher.observe(30, 1).forward);
    }

    #[test]
    fn modifier_combinations_must_match_exactly() {
        let mut matcher = BindingMatcher::new(vec![binding("Shift+Super+R", false)]);
        matcher.observe(keycodes::KEY_LEFTMETA, 1);
        assert!(matcher.observe(19, 1).forward);
        matcher.observe(keycodes::KEY_LEFTSHIFT, 1);
        assert!(!matcher.observe(19, 1).forward);
    }

    #[test]
    fn left_and_right_modifier_keys_are_equivalent() {
        let mut left = BindingMatcher::new(vec![binding("Super+Space", false)]);
        left.observe(keycodes::KEY_LEFTMETA, 1);
        assert!(!left.observe(keycodes::KEY_SPACE, 1).forward);

        let mut right = BindingMatcher::new(vec![binding("Super+Space", false)]);
        right.observe(keycodes::KEY_RIGHTMETA, 1);
        assert!(!right.observe(keycodes::KEY_SPACE, 1).forward);
    }

    #[test]
    fn reload_stops_active_binding_and_keeps_release_suppressed() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Space", true)]);
        matcher.observe(keycodes::KEY_LEFTMETA, 1);
        matcher.observe(keycodes::KEY_SPACE, 1);

        let stopped = matcher.reload(Vec::new());
        let release = matcher.observe(keycodes::KEY_SPACE, 0);

        assert_eq!(stopped.len(), 1);
        assert_eq!(stopped[0].phase, Phase::STOP);
        assert!(!release.forward);
        assert!(release.events.is_empty());
        assert_eq!(
            matcher.state.pressed_modifiers(),
            BTreeSet::from([Modifier::Super])
        );
    }

    #[test]
    fn duplicate_press_of_bound_key_never_dispatches_twice() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Down", false)]);
        matcher.observe(keycodes::KEY_LEFTMETA, 1);

        let first = matcher.observe(keycodes::KEY_DOWN, 1);
        let duplicate = matcher.observe(keycodes::KEY_DOWN, 1);
        let release = matcher.observe(keycodes::KEY_DOWN, 0);

        assert!(!first.forward);
        assert_eq!(first.events.len(), 1);
        assert_eq!(first.events[0].phase, Phase::START);
        assert!(!duplicate.forward);
        assert!(duplicate.events.is_empty(), "duplicate press re-dispatched");
        assert!(!release.forward);
        assert!(release.events.is_empty());
    }

    #[test]
    fn duplicate_press_of_plain_key_is_not_forwarded_twice() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Down", false)]);
        let first = matcher.observe(keycodes::KEY_ENTER, 1);
        let duplicate = matcher.observe(keycodes::KEY_ENTER, 1);
        let release = matcher.observe(keycodes::KEY_ENTER, 0);

        assert!(first.forward);
        assert!(!duplicate.forward);
        assert!(duplicate.events.is_empty());
        assert!(release.forward);
    }

    #[test]
    fn duplicate_modifier_press_is_not_forwarded_twice() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Down", false)]);
        let first = matcher.observe(keycodes::KEY_LEFTMETA, 1);
        let duplicate = matcher.observe(keycodes::KEY_LEFTMETA, 1);
        let release = matcher.observe(keycodes::KEY_LEFTMETA, 0);

        assert!(first.forward);
        assert!(!duplicate.forward);
        assert!(release.forward);
    }

    #[test]
    fn seeded_modifier_dispatches_fresh_combo_press_without_its_press_event() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Down", false)]);
        matcher.seed_held([keycodes::KEY_LEFTMETA]);

        let press = matcher.observe(keycodes::KEY_DOWN, 1);
        let release = matcher.observe(keycodes::KEY_DOWN, 0);
        let super_release = matcher.observe(keycodes::KEY_LEFTMETA, 0);

        assert!(!press.forward);
        assert_eq!(press.events[0].phase, Phase::START);
        assert!(!release.forward);
        assert!(super_release.forward);
    }

    #[test]
    fn seeded_held_bound_key_duplicate_press_is_suppressed_and_release_forwarded() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Down", false)]);
        matcher.seed_held([keycodes::KEY_DOWN]);

        let duplicate = matcher.observe(keycodes::KEY_DOWN, 1);
        let release = matcher.observe(keycodes::KEY_DOWN, 0);

        assert!(!duplicate.forward);
        assert!(duplicate.events.is_empty());
        assert!(release.forward);
    }

    #[test]
    fn reconcile_replay_press_is_state_not_dispatch_and_enables_fresh_combo() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Down", false)]);
        matcher.reconcile(keycodes::KEY_LEFTMETA, 1);

        let replay = matcher.observe(keycodes::KEY_DOWN, 1);
        assert!(!replay.forward);
        assert_eq!(
            replay.events.len(),
            1,
            "replayed key must dispatch as a fresh combo"
        );

        let fresh = matcher.observe(keycodes::KEY_LEFTMETA, 1);
        assert!(
            !fresh.forward,
            "replayed duplicate modifier press must not forward"
        );
        let replayed = matcher.observe(keycodes::KEY_DOWN, 1);
        assert!(!replayed.forward);
        assert!(
            replayed.events.is_empty(),
            "duplicate combo press re-dispatched"
        );
    }

    #[test]
    fn reconcile_release_stops_active_continuous_binding() {
        let mut matcher = BindingMatcher::new(vec![binding("Ctrl+Alt+Shift+Left", true)]);
        matcher.observe(keycodes::KEY_LEFTCTRL, 1);
        matcher.observe(keycodes::KEY_LEFTALT, 1);
        matcher.observe(keycodes::KEY_LEFTSHIFT, 1);

        let start = matcher.observe(keycodes::KEY_LEFT, 1);
        let stop = matcher.reconcile(keycodes::KEY_LEFT, 0);

        assert_eq!(start.events[0].phase, Phase::START);
        assert_eq!(stop.len(), 1);
        assert_eq!(stop[0].phase, Phase::STOP);
        assert!(matcher.active_continuous.is_empty());
        assert!(!matcher.suppressed_keys.contains(&keycodes::KEY_LEFT));
    }

    #[test]
    fn reconcile_release_without_active_continuous_emits_nothing() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Down", false)]);
        matcher.reconcile(keycodes::KEY_LEFTMETA, 1);

        let stopped = matcher.reconcile(keycodes::KEY_LEFTMETA, 0);

        assert!(stopped.is_empty());
        assert!(matcher.state.pressed_modifiers().is_empty());
    }

    #[test]
    fn reconcile_release_clears_seeded_modifier() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Down", false)]);
        matcher.reconcile(keycodes::KEY_LEFTMETA, 1);
        matcher.reconcile(keycodes::KEY_LEFTMETA, 0);

        let press = matcher.observe(keycodes::KEY_DOWN, 1);
        assert!(press.forward);
        assert!(press.events.is_empty());
    }
}
