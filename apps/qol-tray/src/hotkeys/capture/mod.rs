//! Pluggable hotkey capture backends.
//!
//! Today qol-tray uses the `global_hotkey` crate (XGrabKey on Linux), which
//! loses silently when another X11 client holds a passive grab on the same
//! combo (csd-keyboard for `<Super>space`, IBus for input-source switching,
//! etc.). The Linux `evdev` backend in this module reads `/dev/input/event*`
//! and re-emits via `/dev/uinput`, capturing keys before X11 ever sees them.
//!
//! The pure-logic types in this module — `Mod`, `Combo`, `ModifierState`,
//! `BindingMatcher` — are backend-agnostic and unit-testable without any
//! kernel access.

mod binding;
mod modstate;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::install;
#[cfg(target_os = "macos")]
pub(crate) use macos::install;
#[cfg(target_os = "windows")]
pub(crate) use windows::install;

#[cfg(test)]
pub(crate) use binding::Mod;
pub(crate) use binding::{parse_combo, Binding, Combo};
pub(crate) use modstate::ModifierState;

/// Linux evdev keycode constants used by the pure-logic layer so it does not
/// depend on the `evdev` crate. Values match `linux/input-event-codes.h`.
pub(crate) mod keycodes {
    pub(crate) const KEY_ESC: u16 = 1;
    pub(crate) const KEY_BACKSPACE: u16 = 14;
    pub(crate) const KEY_TAB: u16 = 15;
    pub(crate) const KEY_ENTER: u16 = 28;
    pub(crate) const KEY_LEFTCTRL: u16 = 29;
    pub(crate) const KEY_LEFTSHIFT: u16 = 42;
    pub(crate) const KEY_RIGHTSHIFT: u16 = 54;
    pub(crate) const KEY_LEFTALT: u16 = 56;
    pub(crate) const KEY_SPACE: u16 = 57;
    pub(crate) const KEY_F1: u16 = 59;
    pub(crate) const KEY_F12: u16 = 88;
    pub(crate) const KEY_PRINTSCREEN: u16 = 99;
    pub(crate) const KEY_RIGHTCTRL: u16 = 97;
    pub(crate) const KEY_RIGHTALT: u16 = 100;
    pub(crate) const KEY_HOME: u16 = 102;
    pub(crate) const KEY_UP: u16 = 103;
    pub(crate) const KEY_PAGEUP: u16 = 104;
    pub(crate) const KEY_LEFT: u16 = 105;
    pub(crate) const KEY_RIGHT: u16 = 106;
    pub(crate) const KEY_END: u16 = 107;
    pub(crate) const KEY_DOWN: u16 = 108;
    pub(crate) const KEY_PAGEDOWN: u16 = 109;
    pub(crate) const KEY_INSERT: u16 = 110;
    pub(crate) const KEY_DELETE: u16 = 111;
    pub(crate) const KEY_PAUSE: u16 = 119;
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

/// Outcome of feeding one key event to the matcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CaptureDecision {
    /// Forward the event to the virtual device. (Always do this for modifier
    /// events and for non-press events.)
    Forward,
    /// Suppress the event AND fire this binding's action.
    Fire(Binding),
}

#[derive(Debug, Default)]
pub(crate) struct BindingMatcher {
    bindings: Vec<(Combo, Binding)>,
    state: ModifierState,
}

impl BindingMatcher {
    pub(crate) fn new(bindings: Vec<Binding>) -> Self {
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
    pub(crate) fn observe(&mut self, code: u16, value: i32) -> CaptureDecision {
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
    pub(crate) fn referenced_keycodes(&self) -> std::collections::BTreeSet<u16> {
        let mut codes = std::collections::BTreeSet::new();
        for (combo, _) in &self.bindings {
            codes.insert(combo.key);
            for m in &combo.mods {
                for code in m.evdev_codes() {
                    codes.insert(code);
                }
            }
        }
        codes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

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
