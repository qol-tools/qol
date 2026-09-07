use std::collections::BTreeSet;

use crate::grammar::{Key, Modifier, NamedKey};

pub const KEY_ESC: u16 = 1;
const KEY_MINUS: u16 = 12;
const KEY_EQUAL: u16 = 13;
const KEY_LEFTBRACE: u16 = 26;
const KEY_RIGHTBRACE: u16 = 27;
const KEY_SEMICOLON: u16 = 39;
const KEY_APOSTROPHE: u16 = 40;
const KEY_GRAVE: u16 = 41;
const KEY_BACKSLASH: u16 = 43;
const KEY_COMMA: u16 = 51;
const KEY_DOT: u16 = 52;
const KEY_SLASH: u16 = 53;
pub const KEY_BACKSPACE: u16 = 14;
pub const KEY_TAB: u16 = 15;
pub const KEY_ENTER: u16 = 28;
pub const KEY_LEFTCTRL: u16 = 29;
pub const KEY_LEFTSHIFT: u16 = 42;
pub const KEY_RIGHTSHIFT: u16 = 54;
pub const KEY_LEFTALT: u16 = 56;
pub const KEY_SPACE: u16 = 57;
pub const KEY_F1: u16 = 59;
pub const KEY_F12: u16 = 88;
pub const KEY_RIGHTCTRL: u16 = 97;
pub const KEY_PRINTSCREEN: u16 = 99;
pub const KEY_RIGHTALT: u16 = 100;
pub const KEY_HOME: u16 = 102;
pub const KEY_UP: u16 = 103;
pub const KEY_PAGEUP: u16 = 104;
pub const KEY_LEFT: u16 = 105;
pub const KEY_RIGHT: u16 = 106;
pub const KEY_END: u16 = 107;
pub const KEY_DOWN: u16 = 108;
pub const KEY_PAGEDOWN: u16 = 109;
pub const KEY_INSERT: u16 = 110;
pub const KEY_DELETE: u16 = 111;
pub const KEY_PAUSE: u16 = 119;
pub const KEY_LEFTMETA: u16 = 125;
pub const KEY_RIGHTMETA: u16 = 126;

pub const MODIFIER_KEYCODES: [u16; 8] = [
    KEY_LEFTSHIFT,
    KEY_RIGHTSHIFT,
    KEY_LEFTCTRL,
    KEY_RIGHTCTRL,
    KEY_LEFTALT,
    KEY_RIGHTALT,
    KEY_LEFTMETA,
    KEY_RIGHTMETA,
];

pub fn modifier_keycodes(modifier: Modifier) -> [u16; 2] {
    match modifier {
        Modifier::Shift => [KEY_LEFTSHIFT, KEY_RIGHTSHIFT],
        Modifier::Ctrl => [KEY_LEFTCTRL, KEY_RIGHTCTRL],
        Modifier::Alt => [KEY_LEFTALT, KEY_RIGHTALT],
        Modifier::Super => [KEY_LEFTMETA, KEY_RIGHTMETA],
    }
}

pub fn modifier_for_keycode(keycode: u16) -> Option<Modifier> {
    Some(match keycode {
        KEY_LEFTSHIFT | KEY_RIGHTSHIFT => Modifier::Shift,
        KEY_LEFTCTRL | KEY_RIGHTCTRL => Modifier::Ctrl,
        KEY_LEFTALT | KEY_RIGHTALT => Modifier::Alt,
        KEY_LEFTMETA | KEY_RIGHTMETA => Modifier::Super,
        _ => return None,
    })
}

pub fn is_modifier_keycode(keycode: u16) -> bool {
    modifier_for_keycode(keycode).is_some()
}

/// Tracks every physical modifier key independently so releasing one side does
/// not clear a modifier while its other side is still held.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ModifierState {
    pressed_keycodes: BTreeSet<u16>,
}

impl ModifierState {
    /// Applies an evdev key value and returns whether `keycode` is a modifier.
    /// Values follow evdev semantics: 0 is release, 1 is press, and 2 is repeat.
    pub fn handle(&mut self, keycode: u16, value: i32) -> bool {
        if !is_modifier_keycode(keycode) {
            return false;
        }
        if value == 2 {
            return true;
        }
        if value == 1 {
            self.pressed_keycodes.insert(keycode);
        } else {
            self.pressed_keycodes.remove(&keycode);
        }
        true
    }

    pub fn pressed_modifiers(&self) -> BTreeSet<Modifier> {
        [
            Modifier::Ctrl,
            Modifier::Alt,
            Modifier::Shift,
            Modifier::Super,
        ]
        .into_iter()
        .filter(|modifier| {
            modifier_keycodes(*modifier)
                .iter()
                .any(|keycode| self.pressed_keycodes.contains(keycode))
        })
        .collect()
    }
}

pub fn key_to_keycode(key: Key) -> Option<u16> {
    Some(match key {
        Key::Letter(index) => *LETTERS.get(index as usize)?,
        Key::Digit(index) => {
            if index > 9 {
                return None;
            } else if index == 0 {
                11
            } else {
                1 + index as u16
            }
        }
        Key::Function(number) => match number {
            11 => 87,
            12 => KEY_F12,
            1..=10 => KEY_F1 + (number as u16 - 1),
            _ => return None,
        },
        Key::Named(named) => named_to_keycode(named),
        Key::Symbol(symbol) => symbol_to_keycode(symbol)?,
    })
}

fn symbol_to_keycode(symbol: char) -> Option<u16> {
    Some(match symbol {
        '-' => KEY_MINUS,
        '=' => KEY_EQUAL,
        '[' => KEY_LEFTBRACE,
        ']' => KEY_RIGHTBRACE,
        ';' => KEY_SEMICOLON,
        '\'' => KEY_APOSTROPHE,
        '`' => KEY_GRAVE,
        '\\' => KEY_BACKSLASH,
        ',' => KEY_COMMA,
        '.' => KEY_DOT,
        '/' => KEY_SLASH,
        _ => return None,
    })
}

fn named_to_keycode(named: NamedKey) -> u16 {
    match named {
        NamedKey::Space => KEY_SPACE,
        NamedKey::Enter => KEY_ENTER,
        NamedKey::Escape => KEY_ESC,
        NamedKey::Tab => KEY_TAB,
        NamedKey::Backspace => KEY_BACKSPACE,
        NamedKey::Delete => KEY_DELETE,
        NamedKey::Insert => KEY_INSERT,
        NamedKey::Home => KEY_HOME,
        NamedKey::End => KEY_END,
        NamedKey::PageUp => KEY_PAGEUP,
        NamedKey::PageDown => KEY_PAGEDOWN,
        NamedKey::Up => KEY_UP,
        NamedKey::Down => KEY_DOWN,
        NamedKey::Left => KEY_LEFT,
        NamedKey::Right => KEY_RIGHT,
        NamedKey::PrintScreen => KEY_PRINTSCREEN,
        NamedKey::Pause => KEY_PAUSE,
    }
}

const LETTERS: [u16; 26] = [
    30, 48, 46, 32, 18, 33, 34, 35, 23, 36, 37, 38, 50, 49, 24, 25, 16, 19, 31, 20, 22, 47, 17, 45,
    21, 44,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar;

    fn code(input: &str) -> Option<u16> {
        key_to_keycode(grammar::parse(input).unwrap().key)
    }

    #[test]
    fn maps_letters_digits_and_functions() {
        assert_eq!(code("r"), Some(19));
        assert_eq!(code("1"), Some(2));
        assert_eq!(code("9"), Some(10));
        assert_eq!(code("0"), Some(11));
        assert_eq!(code("f1"), Some(KEY_F1));
        assert_eq!(code("f12"), Some(KEY_F12));
    }

    #[test]
    fn maps_named_keys() {
        assert_eq!(code("space"), Some(KEY_SPACE));
        assert_eq!(code("tab"), Some(KEY_TAB));
        assert_eq!(code("up"), Some(KEY_UP));
        assert_eq!(code("down"), Some(KEY_DOWN));
        assert_eq!(code("left"), Some(KEY_LEFT));
        assert_eq!(code("right"), Some(KEY_RIGHT));
    }

    #[test]
    fn maps_only_the_symbols_that_hold_a_physical_position() {
        assert_eq!(code("-"), Some(KEY_MINUS));
        assert_eq!(code(","), Some(KEY_COMMA));
        assert_eq!(code("/"), Some(KEY_SLASH));
        assert_eq!(key_to_keycode(Key::Symbol('+')), None);
        assert_eq!(key_to_keycode(Key::Symbol('\u{e5}')), None);
    }

    #[test]
    fn rejects_out_of_range_public_key_values() {
        assert_eq!(key_to_keycode(Key::Letter(26)), None);
        assert_eq!(key_to_keycode(Key::Digit(10)), None);
        assert_eq!(key_to_keycode(Key::Function(13)), None);
    }

    #[test]
    fn modifier_keycodes_round_trip_and_are_unique() {
        let mut seen = BTreeSet::new();
        for modifier in [
            Modifier::Ctrl,
            Modifier::Alt,
            Modifier::Shift,
            Modifier::Super,
        ] {
            for keycode in modifier_keycodes(modifier) {
                assert_eq!(modifier_for_keycode(keycode), Some(modifier));
                assert!(seen.insert(keycode), "duplicate keycode {keycode}");
            }
        }
        assert_eq!(seen, BTreeSet::from(MODIFIER_KEYCODES));
    }

    #[test]
    fn modifier_state_tracks_left_and_right_keys_independently() {
        let mut state = ModifierState::default();
        state.handle(KEY_LEFTSHIFT, 1);
        state.handle(KEY_RIGHTSHIFT, 1);
        state.handle(KEY_LEFTSHIFT, 0);
        assert_eq!(state.pressed_modifiers(), BTreeSet::from([Modifier::Shift]));
        state.handle(KEY_RIGHTSHIFT, 0);
        assert!(state.pressed_modifiers().is_empty());
    }

    #[test]
    fn modifier_state_handles_combinations_repeats_and_non_modifiers() {
        let mut state = ModifierState::default();
        assert!(state.handle(KEY_LEFTSHIFT, 1));
        assert!(state.handle(KEY_LEFTCTRL, 1));
        assert!(state.handle(KEY_RIGHTMETA, 1));
        assert!(state.handle(KEY_LEFTSHIFT, 2));
        assert!(!state.handle(KEY_SPACE, 1));
        assert_eq!(
            state.pressed_modifiers(),
            BTreeSet::from([Modifier::Shift, Modifier::Ctrl, Modifier::Super])
        );
    }
}
