use qol_hotkeys::macos_keycode::{to_hid_usage, PhysicalLayout};

const LETTERS: [&str; 26] = [
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s",
    "t", "u", "v", "w", "x", "y", "z",
];
const DIGITS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
const FUNCTION_KEYS: [&str; 12] = [
    "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12",
];
const KEYPAD_DIGITS: [&str; 10] = [
    "keypad_1", "keypad_2", "keypad_3", "keypad_4", "keypad_5", "keypad_6", "keypad_7", "keypad_8",
    "keypad_9", "keypad_0",
];

pub(super) fn for_keycode(keycode: u16, layout: PhysicalLayout) -> Option<&'static str> {
    for_usage(to_hid_usage(keycode, layout)?)
}

pub(super) fn for_usage(usage: u16) -> Option<&'static str> {
    Some(match usage {
        0x04..=0x1D => LETTERS[usize::from(usage - 0x04)],
        0x1E..=0x27 => DIGITS[usize::from(usage - 0x1E)],
        0x28 => "return_or_enter",
        0x29 => "escape",
        0x2A => "delete_or_backspace",
        0x2B => "tab",
        0x2C => "spacebar",
        0x2D => "hyphen",
        0x2E => "equal_sign",
        0x2F => "open_bracket",
        0x30 => "close_bracket",
        0x31 => "backslash",
        0x32 => "non_us_pound",
        0x33 => "semicolon",
        0x34 => "quote",
        0x35 => "grave_accent_and_tilde",
        0x36 => "comma",
        0x37 => "period",
        0x38 => "slash",
        0x39 => "caps_lock",
        0x3A..=0x45 => FUNCTION_KEYS[usize::from(usage - 0x3A)],
        0x4A => "home",
        0x4B => "page_up",
        0x4C => "delete_forward",
        0x4D => "end",
        0x4E => "page_down",
        0x4F => "right_arrow",
        0x50 => "left_arrow",
        0x51 => "down_arrow",
        0x52 => "up_arrow",
        0x53 => "keypad_num_lock",
        0x54 => "keypad_slash",
        0x55 => "keypad_asterisk",
        0x56 => "keypad_hyphen",
        0x57 => "keypad_plus",
        0x58 => "keypad_enter",
        0x59..=0x62 => KEYPAD_DIGITS[usize::from(usage - 0x59)],
        0x63 => "keypad_period",
        0x64 => "non_us_backslash",
        0x67 => "keypad_equal_sign",
        0xE0 => "left_control",
        0xE1 => "left_shift",
        0xE2 => "left_option",
        0xE3 => "left_command",
        0xE4 => "right_control",
        0xE5 => "right_shift",
        0xE6 => "right_option",
        0xE7 => "right_command",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use qol_hotkeys::macos_keycode::{self as keycode, PhysicalLayout};

    use super::*;

    #[test]
    fn keycodes_name_the_key_karabiner_knows() {
        for (code, name) in [
            (keycode::ANSI_A, "a"),
            (keycode::ANSI_Z, "z"),
            (keycode::ANSI_1, "1"),
            (keycode::ANSI_0, "0"),
            (keycode::RETURN, "return_or_enter"),
            (keycode::DELETE, "delete_or_backspace"),
            (keycode::FORWARD_DELETE, "delete_forward"),
            (keycode::LEFT_ARROW, "left_arrow"),
            (keycode::ANSI_RIGHT_BRACKET, "close_bracket"),
            (keycode::F12, "f12"),
            (keycode::SPACE, "spacebar"),
        ] {
            assert_eq!(
                for_keycode(code, PhysicalLayout::Ansi),
                Some(name),
                "keycode {code:#x}"
            );
        }
    }

    #[test]
    fn iso_keyboards_name_the_physical_key() {
        assert_eq!(
            for_keycode(keycode::ISO_SECTION, PhysicalLayout::Iso),
            for_keycode(keycode::ANSI_GRAVE, PhysicalLayout::Ansi),
        );
    }

    #[test]
    fn usages_outside_the_table_have_no_name() {
        assert_eq!(for_usage(0x00), None);
        assert_eq!(for_usage(0x46), None);
        assert_eq!(for_usage(0xE8), None);
    }
}
