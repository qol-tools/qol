use super::super::super::binding::{Binding, Combo};
use qol_hotkeys::evdev;
use std::collections::BTreeSet;

/// Outcome of feeding one key event to the matcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum CaptureDecision {
    /// Forward the event to the virtual device. (Always do this for modifier
    /// events and for non-press events.)
    Forward,
    /// Suppress the event AND fire this binding's action.
    Fire(Binding),
}

#[derive(Debug, Default)]
pub(super) struct BindingMatcher {
    bindings: Vec<(Combo, Binding)>,
    state: evdev::ModifierState,
}

impl BindingMatcher {
    pub(super) fn new(bindings: Vec<Binding>) -> Self {
        let bindings = bindings
            .into_iter()
            .filter_map(|b| b.combo.clone().map(|c| (c, b)))
            .collect();
        Self {
            bindings,
            state: evdev::ModifierState::default(),
        }
    }

    /// Process one key event. `value`: 0 = release, 1 = press, 2 = repeat.
    pub(super) fn observe(&mut self, code: u16, value: i32) -> CaptureDecision {
        if self.state.handle(code, value) {
            return CaptureDecision::Forward;
        }
        if value != 1 {
            return CaptureDecision::Forward;
        }
        let current = self.state.pressed_modifiers();
        for (combo, binding) in &self.bindings {
            if combo.key == code && combo.mods == current {
                return CaptureDecision::Fire(binding.clone());
            }
        }
        CaptureDecision::Forward
    }

    /// All evdev keycodes referenced by any binding (including modifiers used).
    /// Useful for sizing the uinput `with_keys` capability set.
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

#[cfg(test)]
mod matcher_tests {
    use super::*;
    use crate::hotkeys::capture::parse_combo;
    use qol_hotkeys::evdev as keycodes;
    use qol_hotkeys::grammar::Modifier;

    fn binding(key_str: &str, plugin: &str, action: &str) -> Binding {
        Binding {
            combo: parse_combo(key_str),
            plugin_uid: crate::plugins::PluginUid::new(plugin),
            action: action.into(),
            raw_key: key_str.into(),
        }
    }

    #[test]
    fn modifier_press_forwards_and_updates_state() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Space", "p", "open")]);
        assert_eq!(
            matcher.observe(keycodes::KEY_LEFTMETA, 1),
            CaptureDecision::Forward
        );
        assert_eq!(
            matcher.state.pressed_modifiers(),
            BTreeSet::from([Modifier::Super])
        );
    }

    #[test]
    fn matched_combo_fires_on_press_and_is_suppressed() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Space", "p", "open")]);
        matcher.observe(keycodes::KEY_LEFTMETA, 1);
        match matcher.observe(keycodes::KEY_SPACE, 1) {
            CaptureDecision::Fire(b) => {
                assert_eq!(b.plugin_uid.as_str(), "p");
                assert_eq!(b.action, "open");
            }
            other => panic!("expected Fire, got {other:?}"),
        }
    }

    #[test]
    fn key_release_always_forwards_even_for_bound_key() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Space", "p", "open")]);
        matcher.observe(keycodes::KEY_LEFTMETA, 1);
        assert_eq!(
            matcher.observe(keycodes::KEY_SPACE, 0),
            CaptureDecision::Forward
        );
    }

    #[test]
    fn key_repeat_does_not_refire() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Space", "p", "open")]);
        matcher.observe(keycodes::KEY_LEFTMETA, 1);
        matcher.observe(keycodes::KEY_SPACE, 1);
        assert_eq!(
            matcher.observe(keycodes::KEY_SPACE, 2),
            CaptureDecision::Forward
        );
    }

    #[test]
    fn unmatched_key_with_modifiers_forwards() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Space", "p", "open")]);
        matcher.observe(keycodes::KEY_LEFTMETA, 1);
        assert_eq!(matcher.observe(30, 1), CaptureDecision::Forward);
    }

    #[test]
    fn modifier_combinations_must_match_exactly() {
        let mut matcher = BindingMatcher::new(vec![binding("Shift+Super+R", "p", "open")]);
        matcher.observe(keycodes::KEY_LEFTMETA, 1);
        assert_eq!(matcher.observe(19, 1), CaptureDecision::Forward);
        matcher.observe(keycodes::KEY_LEFTSHIFT, 1);
        match matcher.observe(19, 1) {
            CaptureDecision::Fire(_) => {}
            other => panic!("expected Fire, got {other:?}"),
        }
    }

    #[test]
    fn left_and_right_modifier_keys_are_equivalent() {
        let mut left = BindingMatcher::new(vec![binding("Super+Space", "p", "open")]);
        left.observe(keycodes::KEY_LEFTMETA, 1);
        assert!(matches!(
            left.observe(keycodes::KEY_SPACE, 1),
            CaptureDecision::Fire(_)
        ));

        let mut right = BindingMatcher::new(vec![binding("Super+Space", "p", "open")]);
        right.observe(keycodes::KEY_RIGHTMETA, 1);
        assert!(matches!(
            right.observe(keycodes::KEY_SPACE, 1),
            CaptureDecision::Fire(_)
        ));
    }

    #[test]
    fn referenced_keycodes_includes_combo_key_and_modifiers() {
        let matcher = BindingMatcher::new(vec![
            binding("Super+Space", "p", "open"),
            binding("Shift+Super+R", "p", "alt"),
        ]);
        let codes = matcher.referenced_keycodes();
        assert!(codes.contains(&keycodes::KEY_SPACE));
        assert!(codes.contains(&keycodes::KEY_LEFTMETA));
        assert!(codes.contains(&keycodes::KEY_RIGHTMETA));
        assert!(codes.contains(&keycodes::KEY_LEFTSHIFT));
        assert!(codes.contains(&keycodes::KEY_RIGHTSHIFT));
        assert!(codes.contains(&19));
    }
}
