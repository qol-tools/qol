use super::binding::Mod;
use super::keycodes;
use std::collections::BTreeSet;

/// Tracks the press state of every modifier key, separately for left/right
/// physical keys, so that a release on one side does not falsely mark the
/// modifier as up while the other side is still held.
#[derive(Debug, Default, Clone)]
pub(crate) struct ModifierState {
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
    pub(crate) fn handle(&mut self, code: u16, value: i32) {
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

    pub(crate) fn current_mods(&self) -> BTreeSet<Mod> {
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

#[cfg(test)]
mod tests {
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
