use crate::plugins::PluginUid;
use qol_hotkeys::evdev;
use qol_hotkeys::grammar::{self, Modifier};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Combo {
    pub(crate) mods: BTreeSet<Modifier>,
    pub(crate) key: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    pub(crate) combo: Option<Combo>,
    pub(crate) plugin_uid: PluginUid,
    pub(crate) action: String,
    pub(crate) raw_key: String,
    pub(crate) continuous: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Start,
    Heartbeat,
    Stop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CaptureEvent {
    pub(crate) binding: Binding,
    pub(crate) phase: Phase,
}

/// Parse a qol-tray combo string ("Super+Space", "Shift+Super+R", "Ctrl+Alt+Shift+F12")
/// into a `Combo` of evdev keycodes. Returns `None` for unknown keys or
/// modifier-only inputs.
pub(crate) fn parse_combo(input: &str) -> Option<Combo> {
    let parsed = grammar::parse(input)?;
    Some(Combo {
        mods: parsed.mods,
        key: evdev::key_to_keycode(parsed.key)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_hotkeys::evdev as keycodes;

    #[test]
    fn parses_super_space() {
        let combo = parse_combo("Super+Space").unwrap();
        assert_eq!(combo.mods, BTreeSet::from([Modifier::Super]));
        assert_eq!(combo.key, keycodes::KEY_SPACE);
    }

    #[test]
    fn parses_shift_super_r() {
        let combo = parse_combo("Shift+Super+R").unwrap();
        assert_eq!(
            combo.mods,
            BTreeSet::from([Modifier::Shift, Modifier::Super])
        );
        assert_eq!(combo.key, 19);
    }

    #[test]
    fn parses_ctrl_alt_shift_f12() {
        let combo = parse_combo("Ctrl+Alt+Shift+F12").unwrap();
        assert_eq!(
            combo.mods,
            BTreeSet::from([Modifier::Ctrl, Modifier::Alt, Modifier::Shift])
        );
        assert_eq!(combo.key, keycodes::KEY_F12);
    }

    #[test]
    fn parses_alt_tab() {
        let combo = parse_combo("Alt+Tab").unwrap();
        assert_eq!(combo.mods, BTreeSet::from([Modifier::Alt]));
        assert_eq!(combo.key, keycodes::KEY_TAB);
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(parse_combo("super+space"), parse_combo("Super+Space"));
        assert_eq!(parse_combo("SHIFT+SUPER+r"), parse_combo("Shift+Super+R"));
    }

    #[test]
    fn rejects_modifier_only() {
        assert!(parse_combo("Shift").is_none());
        assert!(parse_combo("Ctrl+Alt").is_none());
    }

    #[test]
    fn rejects_two_non_modifier_keys() {
        assert!(parse_combo("Space+Enter").is_none());
    }

    #[test]
    fn rejects_unknown_key() {
        assert!(parse_combo("Super+Foo").is_none());
    }

    #[test]
    fn function_keys_in_range() {
        assert_eq!(parse_combo("F1").unwrap().key, keycodes::KEY_F1);
        assert_eq!(parse_combo("F12").unwrap().key, keycodes::KEY_F12);
        assert!(parse_combo("F13").is_none());
        assert!(parse_combo("F0").is_none());
    }

    #[test]
    fn arrow_keys() {
        assert_eq!(parse_combo("Super+Up").unwrap().key, keycodes::KEY_UP);
        assert_eq!(parse_combo("Super+Down").unwrap().key, keycodes::KEY_DOWN);
        assert_eq!(parse_combo("Super+Left").unwrap().key, keycodes::KEY_LEFT);
        assert_eq!(parse_combo("Super+Right").unwrap().key, keycodes::KEY_RIGHT);
    }

    #[test]
    fn digits() {
        assert_eq!(parse_combo("Super+1").unwrap().key, 2);
        assert_eq!(parse_combo("Super+9").unwrap().key, 10);
        assert_eq!(parse_combo("Super+0").unwrap().key, 11);
    }

    #[test]
    fn modifier_canonicalization_via_set() {
        let a = parse_combo("Ctrl+Alt+Shift+Down").unwrap();
        let b = parse_combo("Shift+Alt+Ctrl+Down").unwrap();
        assert_eq!(a.mods, b.mods);
        assert_eq!(a.key, b.key);
    }
}
