use crate::grammar::{Key, NamedKey};

pub const KEY_ESC: u16 = 1;
pub const KEY_BACKSPACE: u16 = 14;
pub const KEY_TAB: u16 = 15;
pub const KEY_ENTER: u16 = 28;
pub const KEY_SPACE: u16 = 57;
pub const KEY_F1: u16 = 59;
pub const KEY_F12: u16 = 88;
pub const KEY_PRINTSCREEN: u16 = 99;
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
    fn rejects_out_of_range_public_key_values() {
        assert_eq!(key_to_keycode(Key::Letter(26)), None);
        assert_eq!(key_to_keycode(Key::Digit(10)), None);
        assert_eq!(key_to_keycode(Key::Function(13)), None);
    }
}
