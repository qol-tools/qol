use super::super::super::binding::{Binding, CaptureEvent, Phase};
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
    active_continuous: HashMap<u16, (LinuxCombo, Binding, Instant)>,
    suppressed_keys: HashSet<u16>,
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
            return self.modifier_decision();
        }
        match value {
            0 => self.release_key(code),
            1 => self.press_key(code),
            2 if self.suppressed_keys.contains(&code) => self.repeat_key(code),
            _ => CaptureDecision::forward(),
        }
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

    pub(super) fn referenced_keycodes(&self) -> BTreeSet<u16> {
        let mut codes = BTreeSet::new();
        for (combo, _) in &self.bindings {
            codes.insert(combo.key);
            for modifier in &combo.mods {
                for code in evdev::modifier_keycodes(*modifier) {
                    codes.insert(code);
                }
            }
        }
        codes
    }
}

fn linux_bindings(bindings: Vec<Binding>) -> Vec<(LinuxCombo, Binding)> {
    bindings
        .into_iter()
        .filter_map(|binding| {
            let combo = binding.combo.as_ref()?;
            let key = evdev::key_to_keycode(combo.key)?;
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
    fn modifier_state_and_referenced_keycodes_cover_binding_inputs() {
        let matcher = BindingMatcher::new(vec![
            binding("Super+Space", false),
            binding("Shift+Super+R", false),
        ]);
        let codes = matcher.referenced_keycodes();

        assert!(codes.contains(&keycodes::KEY_SPACE));
        assert!(codes.contains(&keycodes::KEY_LEFTMETA));
        assert!(codes.contains(&keycodes::KEY_RIGHTMETA));
        assert!(codes.contains(&keycodes::KEY_LEFTSHIFT));
        assert!(codes.contains(&keycodes::KEY_RIGHTSHIFT));
        assert!(codes.contains(&19));
        assert_eq!(
            matcher.state.pressed_modifiers(),
            BTreeSet::<Modifier>::new()
        );
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
}
