//! Linux-only hotkey matcher: pure-logic types consumed by the evdev backend.
//!
//! Lives here (rather than in `capture/mod.rs`) because every consumer of these
//! symbols is gated on `#[cfg(target_os = "linux")] feature = "linux_evdev"`.
//! Keeping them with the consumer keeps the cross-platform `capture/mod.rs`
//! surface tiny and avoids dead-code warnings on macOS / Windows.

use super::super::super::binding::{Binding, Combo, Mod};
use std::collections::BTreeSet;

/// Linux evdev keycode constants used by the matcher and its tests so this
/// module does not depend on the `evdev` crate. Values match
/// `linux/input-event-codes.h`.
///
/// Module visibility is `pub(super)` so the parent (`linux/`) can re-use the
/// table from `evdev_backend.rs`.
pub(super) mod keycodes {
    pub(crate) const KEY_LEFTCTRL: u16 = 29;
    pub(crate) const KEY_LEFTSHIFT: u16 = 42;
    pub(crate) const KEY_RIGHTSHIFT: u16 = 54;
    pub(crate) const KEY_LEFTALT: u16 = 56;
    #[cfg(test)]
    pub(super) const KEY_SPACE: u16 = 57;
    pub(crate) const KEY_RIGHTCTRL: u16 = 97;
    pub(crate) const KEY_RIGHTALT: u16 = 100;
    pub(crate) const KEY_LEFTMETA: u16 = 125;
    pub(crate) const KEY_RIGHTMETA: u16 = 126;

    pub(crate) fn is_modifier(code: u16) -> bool {
        matches!(
            code,
            KEY_LEFTSHIFT
                | KEY_RIGHTSHIFT
                | KEY_LEFTCTRL
                | KEY_RIGHTCTRL
                | KEY_LEFTALT
                | KEY_RIGHTALT
                | KEY_LEFTMETA
                | KEY_RIGHTMETA
        )
    }
}

/// Both left and right evdev keycodes that count as this modifier.
pub(super) fn evdev_codes_for(m: Mod) -> [u16; 2] {
    match m {
        Mod::Shift => [keycodes::KEY_LEFTSHIFT, keycodes::KEY_RIGHTSHIFT],
        Mod::Ctrl => [keycodes::KEY_LEFTCTRL, keycodes::KEY_RIGHTCTRL],
        Mod::Alt => [keycodes::KEY_LEFTALT, keycodes::KEY_RIGHTALT],
        Mod::Super => [keycodes::KEY_LEFTMETA, keycodes::KEY_RIGHTMETA],
    }
}

/// Tracks the press state of every modifier key, separately for left/right
/// physical keys, so that a release on one side does not falsely mark the
/// modifier as up while the other side is still held.
#[derive(Debug, Default, Clone)]
pub(super) struct ModifierState {
    shift_l: bool,
    shift_r: bool,
    ctrl_l: bool,
    ctrl_r: bool,
    alt_l: bool,
    alt_r: bool,
    super_l: bool,
    super_r: bool,
}

impl ModifierState {
    /// Apply one modifier-key event. Non-modifier codes are ignored.
    /// `value`: 0 = release, 1 = press, 2 = repeat (no state change for repeats).
    pub(super) fn handle(&mut self, code: u16, value: i32) {
        if value == 2 {
            return;
        }
        let pressed = value == 1;
        match code {
            keycodes::KEY_LEFTSHIFT => self.shift_l = pressed,
            keycodes::KEY_RIGHTSHIFT => self.shift_r = pressed,
            keycodes::KEY_LEFTCTRL => self.ctrl_l = pressed,
            keycodes::KEY_RIGHTCTRL => self.ctrl_r = pressed,
            keycodes::KEY_LEFTALT => self.alt_l = pressed,
            keycodes::KEY_RIGHTALT => self.alt_r = pressed,
            keycodes::KEY_LEFTMETA => self.super_l = pressed,
            keycodes::KEY_RIGHTMETA => self.super_r = pressed,
            _ => {}
        }
    }

    pub(super) fn current_mods(&self) -> BTreeSet<Mod> {
        let mut set = BTreeSet::new();
        if self.shift_l || self.shift_r {
            set.insert(Mod::Shift);
        }
        if self.ctrl_l || self.ctrl_r {
            set.insert(Mod::Ctrl);
        }
        if self.alt_l || self.alt_r {
            set.insert(Mod::Alt);
        }
        if self.super_l || self.super_r {
            set.insert(Mod::Super);
        }
        set
    }
}

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
    state: ModifierState,
}

impl BindingMatcher {
    pub(super) fn new(bindings: Vec<Binding>) -> Self {
        let bindings = bindings
            .into_iter()
            .filter_map(|b| b.combo.clone().map(|c| (c, b)))
            .collect();
        Self {
            bindings,
            state: ModifierState::default(),
        }
    }

    /// Process one key event. `value`: 0 = release, 1 = press, 2 = repeat.
    pub(super) fn observe(&mut self, code: u16, value: i32) -> CaptureDecision {
        if keycodes::is_modifier(code) {
            self.state.handle(code, value);
            return CaptureDecision::Forward;
        }
        if value != 1 {
            return CaptureDecision::Forward;
        }
        let current = self.state.current_mods();
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
            for m in &combo.mods {
                for code in evdev_codes_for(*m) {
                    codes.insert(code);
                }
            }
        }
        codes
    }
}

#[cfg(test)]
mod modstate_tests {
    use super::*;

    #[test]
    fn empty_when_nothing_pressed() {
        assert!(ModifierState::default().current_mods().is_empty());
    }

    #[test]
    fn single_press_then_release() {
        let mut s = ModifierState::default();
        s.handle(keycodes::KEY_LEFTSHIFT, 1);
        assert_eq!(s.current_mods(), BTreeSet::from([Mod::Shift]));
        s.handle(keycodes::KEY_LEFTSHIFT, 0);
        assert!(s.current_mods().is_empty());
    }

    #[test]
    fn left_release_keeps_modifier_active_when_right_still_held() {
        let mut s = ModifierState::default();
        s.handle(keycodes::KEY_LEFTSHIFT, 1);
        s.handle(keycodes::KEY_RIGHTSHIFT, 1);
        s.handle(keycodes::KEY_LEFTSHIFT, 0);
        assert_eq!(s.current_mods(), BTreeSet::from([Mod::Shift]));
        s.handle(keycodes::KEY_RIGHTSHIFT, 0);
        assert!(s.current_mods().is_empty());
    }

    #[test]
    fn repeat_does_not_change_state() {
        let mut s = ModifierState::default();
        s.handle(keycodes::KEY_LEFTSHIFT, 1);
        let before = s.current_mods();
        s.handle(keycodes::KEY_LEFTSHIFT, 2);
        assert_eq!(s.current_mods(), before);
        // A spurious release-with-value-2 must not turn it off.
        s.handle(keycodes::KEY_LEFTSHIFT, 2);
        assert_eq!(s.current_mods(), before);
    }

    #[test]
    fn multi_modifier_combination_reports_all() {
        let mut s = ModifierState::default();
        s.handle(keycodes::KEY_LEFTSHIFT, 1);
        s.handle(keycodes::KEY_LEFTCTRL, 1);
        s.handle(keycodes::KEY_RIGHTMETA, 1);
        assert_eq!(
            s.current_mods(),
            BTreeSet::from([Mod::Shift, Mod::Ctrl, Mod::Super])
        );
    }

    #[test]
    fn non_modifier_code_is_ignored() {
        let mut s = ModifierState::default();
        s.handle(keycodes::KEY_SPACE, 1);
        s.handle(30 /* KEY_A */, 1);
        assert!(s.current_mods().is_empty());
    }
}

#[cfg(test)]
mod matcher_tests {
    use super::*;
    use crate::hotkeys::capture::parse_combo;

    fn binding(key_str: &str, plugin: &str, action: &str) -> Binding {
        Binding {
            combo: parse_combo(key_str),
            plugin_id: plugin.into(),
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
        assert_eq!(matcher.state.current_mods(), BTreeSet::from([Mod::Super]));
    }

    #[test]
    fn matched_combo_fires_on_press_and_is_suppressed() {
        let mut matcher = BindingMatcher::new(vec![binding("Super+Space", "p", "open")]);
        matcher.observe(keycodes::KEY_LEFTMETA, 1);
        match matcher.observe(keycodes::KEY_SPACE, 1) {
            CaptureDecision::Fire(b) => {
                assert_eq!(b.plugin_id, "p");
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
