use std::collections::BTreeSet;

/// Linux evdev keycode constants used by the cross-platform combo parser.
/// Only non-modifier keys live here; modifier KEY_* constants and the
/// `Mod -> [u16; 2]` mapping live with the Linux matcher
/// (`super::platform::linux::matcher::keycodes`).
mod keycodes {
    pub(super) const KEY_ESC: u16 = 1;
    pub(super) const KEY_BACKSPACE: u16 = 14;
    pub(super) const KEY_TAB: u16 = 15;
    pub(super) const KEY_ENTER: u16 = 28;
    pub(super) const KEY_SPACE: u16 = 57;
    pub(super) const KEY_F1: u16 = 59;
    pub(super) const KEY_F12: u16 = 88;
    pub(super) const KEY_PRINTSCREEN: u16 = 99;
    pub(super) const KEY_HOME: u16 = 102;
    pub(super) const KEY_UP: u16 = 103;
    pub(super) const KEY_PAGEUP: u16 = 104;
    pub(super) const KEY_LEFT: u16 = 105;
    pub(super) const KEY_RIGHT: u16 = 106;
    pub(super) const KEY_END: u16 = 107;
    pub(super) const KEY_DOWN: u16 = 108;
    pub(super) const KEY_PAGEDOWN: u16 = 109;
    pub(super) const KEY_INSERT: u16 = 110;
    pub(super) const KEY_DELETE: u16 = 111;
    pub(super) const KEY_PAUSE: u16 = 119;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Mod {
    Shift,
    Ctrl,
    Alt,
    Super,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Combo {
    pub(crate) mods: BTreeSet<Mod>,
    pub(crate) key: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Binding {
    pub(crate) combo: Option<Combo>,
    pub(crate) plugin_id: String,
    pub(crate) action: String,
    pub(crate) raw_key: String,
}

/// Parse a qol-tray combo string ("Super+Space", "Shift+Super+R", "Ctrl+Alt+Shift+F12")
/// into a `Combo` of evdev keycodes. Returns `None` for unknown keys or
/// modifier-only inputs.
pub(crate) fn parse_combo(input: &str) -> Option<Combo> {
    let mut mods = BTreeSet::new();
    let mut key: Option<u16> = None;
    for raw in input.split('+') {
        let token = raw.trim().to_ascii_lowercase();
        if let Some(m) = parse_modifier(&token) {
            mods.insert(m);
        } else if let Some(k) = key_name_to_code(&token) {
            if key.is_some() {
                return None;
            }
            key = Some(k);
        } else {
            return None;
        }
    }
    Some(Combo { mods, key: key? })
}

fn parse_modifier(token: &str) -> Option<Mod> {
    match token {
        "shift" => Some(Mod::Shift),
        "ctrl" | "control" => Some(Mod::Ctrl),
        "alt" => Some(Mod::Alt),
        "super" | "meta" | "win" | "cmd" => Some(Mod::Super),
        _ => None,
    }
}

/// Map qol-tray's key names to evdev KEY_* codes. Mirrors the entries the
/// existing `global_hotkey` parser supports.
fn key_name_to_code(name: &str) -> Option<u16> {
    if let Some(c) = letter_code(name) {
        return Some(c);
    }
    if let Some(c) = digit_code(name) {
        return Some(c);
    }
    if let Some(c) = function_key_code(name) {
        return Some(c);
    }
    Some(match name {
        "space" => keycodes::KEY_SPACE,
        "enter" | "return" => keycodes::KEY_ENTER,
        "escape" | "esc" => keycodes::KEY_ESC,
        "tab" => keycodes::KEY_TAB,
        "backspace" => keycodes::KEY_BACKSPACE,
        "delete" | "del" => keycodes::KEY_DELETE,
        "insert" | "ins" => keycodes::KEY_INSERT,
        "home" => keycodes::KEY_HOME,
        "end" => keycodes::KEY_END,
        "pageup" | "pgup" => keycodes::KEY_PAGEUP,
        "pagedown" | "pgdn" => keycodes::KEY_PAGEDOWN,
        "up" => keycodes::KEY_UP,
        "down" => keycodes::KEY_DOWN,
        "left" => keycodes::KEY_LEFT,
        "right" => keycodes::KEY_RIGHT,
        "printscreen" | "print" | "prtsc" => keycodes::KEY_PRINTSCREEN,
        "pause" => keycodes::KEY_PAUSE,
        _ => return None,
    })
}

fn letter_code(n: &str) -> Option<u16> {
    if n.len() != 1 {
        return None;
    }
    let c = n.chars().next()?;
    if !c.is_ascii_lowercase() {
        return None;
    }
    const LETTERS: &[u16] = &[
        30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17,
        45, 21, 44,
    ];
    Some(LETTERS[(c as u8 - b'a') as usize])
}

fn digit_code(n: &str) -> Option<u16> {
    if n.len() != 1 {
        return None;
    }
    let c = n.chars().next()?;
    if !c.is_ascii_digit() {
        return None;
    }
    Some(if c == '0' {
        11
    } else {
        2 + (c as u8 - b'1') as u16
    })
}

fn function_key_code(n: &str) -> Option<u16> {
    let rest = n.strip_prefix('f')?;
    let num: u16 = rest.parse().ok()?;
    // F1..F10 are contiguous (59..=68); F11 and F12 are 87 and 88.
    match num {
        1..=10 => Some(keycodes::KEY_F1 + (num - 1)),
        11 => Some(87),
        12 => Some(keycodes::KEY_F12),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_super_space() {
        let combo = parse_combo("Super+Space").unwrap();
        assert_eq!(combo.mods, BTreeSet::from([Mod::Super]));
        assert_eq!(combo.key, keycodes::KEY_SPACE);
    }

    #[test]
    fn parses_shift_super_r() {
        let combo = parse_combo("Shift+Super+R").unwrap();
        assert_eq!(combo.mods, BTreeSet::from([Mod::Shift, Mod::Super]));
        assert_eq!(combo.key, 19);
    }

    #[test]
    fn parses_ctrl_alt_shift_f12() {
        let combo = parse_combo("Ctrl+Alt+Shift+F12").unwrap();
        assert_eq!(
            combo.mods,
            BTreeSet::from([Mod::Ctrl, Mod::Alt, Mod::Shift])
        );
        assert_eq!(combo.key, keycodes::KEY_F12);
    }

    #[test]
    fn parses_alt_tab() {
        let combo = parse_combo("Alt+Tab").unwrap();
        assert_eq!(combo.mods, BTreeSet::from([Mod::Alt]));
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
