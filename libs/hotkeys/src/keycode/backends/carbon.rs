use crate::grammar::{Key, NamedKey};

pub const ANSI_A: u16 = 0x00;
pub const ANSI_S: u16 = 0x01;
pub const ANSI_D: u16 = 0x02;
pub const ANSI_F: u16 = 0x03;
pub const ANSI_H: u16 = 0x04;
pub const ANSI_G: u16 = 0x05;
pub const ANSI_Z: u16 = 0x06;
pub const ANSI_X: u16 = 0x07;
pub const ANSI_C: u16 = 0x08;
pub const ANSI_V: u16 = 0x09;
pub const ISO_SECTION: u16 = 0x0A;
pub const ANSI_B: u16 = 0x0B;
pub const ANSI_Q: u16 = 0x0C;
pub const ANSI_W: u16 = 0x0D;
pub const ANSI_E: u16 = 0x0E;
pub const ANSI_R: u16 = 0x0F;
pub const ANSI_Y: u16 = 0x10;
pub const ANSI_T: u16 = 0x11;
pub const ANSI_1: u16 = 0x12;
pub const ANSI_2: u16 = 0x13;
pub const ANSI_3: u16 = 0x14;
pub const ANSI_4: u16 = 0x15;
pub const ANSI_6: u16 = 0x16;
pub const ANSI_5: u16 = 0x17;
pub const ANSI_EQUAL: u16 = 0x18;
pub const ANSI_9: u16 = 0x19;
pub const ANSI_7: u16 = 0x1A;
pub const ANSI_MINUS: u16 = 0x1B;
pub const ANSI_8: u16 = 0x1C;
pub const ANSI_0: u16 = 0x1D;
pub const ANSI_RIGHT_BRACKET: u16 = 0x1E;
pub const ANSI_O: u16 = 0x1F;
pub const ANSI_U: u16 = 0x20;
pub const ANSI_LEFT_BRACKET: u16 = 0x21;
pub const ANSI_I: u16 = 0x22;
pub const ANSI_P: u16 = 0x23;
pub const ANSI_L: u16 = 0x25;
pub const ANSI_J: u16 = 0x26;
pub const ANSI_QUOTE: u16 = 0x27;
pub const ANSI_K: u16 = 0x28;
pub const ANSI_SEMICOLON: u16 = 0x29;
pub const ANSI_BACKSLASH: u16 = 0x2A;
pub const ANSI_COMMA: u16 = 0x2B;
pub const ANSI_SLASH: u16 = 0x2C;
pub const ANSI_N: u16 = 0x2D;
pub const ANSI_M: u16 = 0x2E;
pub const ANSI_PERIOD: u16 = 0x2F;
pub const ANSI_GRAVE: u16 = 0x32;

pub const RETURN: u16 = 0x24;
pub const TAB: u16 = 0x30;
pub const SPACE: u16 = 0x31;
pub const DELETE: u16 = 0x33;
pub const ESCAPE: u16 = 0x35;
pub const FORWARD_DELETE: u16 = 0x75;

pub const LEFT_ARROW: u16 = 0x7B;
pub const RIGHT_ARROW: u16 = 0x7C;
pub const DOWN_ARROW: u16 = 0x7D;
pub const UP_ARROW: u16 = 0x7E;

pub const HOME: u16 = 0x73;
pub const END: u16 = 0x77;
pub const PAGE_UP: u16 = 0x74;
pub const PAGE_DOWN: u16 = 0x79;

pub const F1: u16 = 0x7A;
pub const F2: u16 = 0x78;
pub const F3: u16 = 0x63;
pub const F4: u16 = 0x76;
pub const F5: u16 = 0x60;
pub const F6: u16 = 0x61;
pub const F7: u16 = 0x62;
pub const F8: u16 = 0x64;
pub const F9: u16 = 0x65;
pub const F10: u16 = 0x6D;
pub const F11: u16 = 0x67;
pub const F12: u16 = 0x6F;

pub fn key_to_keycode(key: Key) -> Option<u16> {
    Some(match key {
        Key::Letter(index) => *LETTERS.get(index as usize)?,
        Key::Digit(index) => *DIGITS.get(index as usize)?,
        Key::Function(number) => *FUNCTION_KEYS.get(number.checked_sub(1)? as usize)?,
        Key::Named(named) => return named_to_keycode(named),
        Key::Symbol(_) => return None,
    })
}

pub fn parse_key(name: &str) -> Option<u16> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "a" => Some(ANSI_A),
        "b" => Some(ANSI_B),
        "c" => Some(ANSI_C),
        "d" => Some(ANSI_D),
        "e" => Some(ANSI_E),
        "f" => Some(ANSI_F),
        "g" => Some(ANSI_G),
        "h" => Some(ANSI_H),
        "i" => Some(ANSI_I),
        "j" => Some(ANSI_J),
        "k" => Some(ANSI_K),
        "l" => Some(ANSI_L),
        "m" => Some(ANSI_M),
        "n" => Some(ANSI_N),
        "o" => Some(ANSI_O),
        "p" => Some(ANSI_P),
        "q" => Some(ANSI_Q),
        "r" => Some(ANSI_R),
        "s" => Some(ANSI_S),
        "t" => Some(ANSI_T),
        "u" => Some(ANSI_U),
        "v" => Some(ANSI_V),
        "w" => Some(ANSI_W),
        "x" => Some(ANSI_X),
        "y" => Some(ANSI_Y),
        "z" => Some(ANSI_Z),
        "0" => Some(ANSI_0),
        "1" => Some(ANSI_1),
        "2" => Some(ANSI_2),
        "3" => Some(ANSI_3),
        "4" => Some(ANSI_4),
        "5" => Some(ANSI_5),
        "6" => Some(ANSI_6),
        "7" => Some(ANSI_7),
        "8" => Some(ANSI_8),
        "9" => Some(ANSI_9),
        "return" | "enter" => Some(RETURN),
        "tab" => Some(TAB),
        "space" => Some(SPACE),
        "backspace" => Some(DELETE),
        "delete" | "del" | "forwarddelete" => Some(FORWARD_DELETE),
        "escape" | "esc" => Some(ESCAPE),
        "left" => Some(LEFT_ARROW),
        "right" => Some(RIGHT_ARROW),
        "down" => Some(DOWN_ARROW),
        "up" => Some(UP_ARROW),
        "home" => Some(HOME),
        "end" => Some(END),
        "pageup" => Some(PAGE_UP),
        "pagedown" => Some(PAGE_DOWN),
        "f1" => Some(F1),
        "f2" => Some(F2),
        "f3" => Some(F3),
        "f4" => Some(F4),
        "f5" => Some(F5),
        "f6" => Some(F6),
        "f7" => Some(F7),
        "f8" => Some(F8),
        "f9" => Some(F9),
        "f10" => Some(F10),
        "f11" => Some(F11),
        "f12" => Some(F12),
        "-" | "minus" => Some(ANSI_MINUS),
        "=" | "equal" | "plus" => Some(ANSI_EQUAL),
        "[" | "leftbracket" => Some(ANSI_LEFT_BRACKET),
        "]" | "rightbracket" => Some(ANSI_RIGHT_BRACKET),
        "\\" | "backslash" => Some(ANSI_BACKSLASH),
        ";" | "semicolon" => Some(ANSI_SEMICOLON),
        "'" | "quote" => Some(ANSI_QUOTE),
        "," | "comma" => Some(ANSI_COMMA),
        "." | "period" => Some(ANSI_PERIOD),
        "/" | "slash" => Some(ANSI_SLASH),
        "`" | "grave" => Some(ANSI_GRAVE),
        "section" | "iso" | "<" | ">" => Some(ISO_SECTION),
        _ => None,
    }
}

pub fn key_name(code: u16) -> &'static str {
    match code {
        ANSI_A => "a",
        ANSI_B => "b",
        ANSI_C => "c",
        ANSI_D => "d",
        ANSI_E => "e",
        ANSI_F => "f",
        ANSI_G => "g",
        ANSI_H => "h",
        ANSI_I => "i",
        ANSI_J => "j",
        ANSI_K => "k",
        ANSI_L => "l",
        ANSI_M => "m",
        ANSI_N => "n",
        ANSI_O => "o",
        ANSI_P => "p",
        ANSI_Q => "q",
        ANSI_R => "r",
        ANSI_S => "s",
        ANSI_T => "t",
        ANSI_U => "u",
        ANSI_V => "v",
        ANSI_W => "w",
        ANSI_X => "x",
        ANSI_Y => "y",
        ANSI_Z => "z",
        ANSI_0 => "0",
        ANSI_1 => "1",
        ANSI_2 => "2",
        ANSI_3 => "3",
        ANSI_4 => "4",
        ANSI_5 => "5",
        ANSI_6 => "6",
        ANSI_7 => "7",
        ANSI_8 => "8",
        ANSI_9 => "9",
        RETURN => "return",
        TAB => "tab",
        SPACE => "space",
        DELETE => "backspace",
        FORWARD_DELETE => "delete",
        ESCAPE => "escape",
        LEFT_ARROW => "left",
        RIGHT_ARROW => "right",
        DOWN_ARROW => "down",
        UP_ARROW => "up",
        HOME => "home",
        END => "end",
        PAGE_UP => "pageup",
        PAGE_DOWN => "pagedown",
        F1 => "f1",
        F2 => "f2",
        F3 => "f3",
        F4 => "f4",
        F5 => "f5",
        F6 => "f6",
        F7 => "f7",
        F8 => "f8",
        F9 => "f9",
        F10 => "f10",
        F11 => "f11",
        F12 => "f12",
        ANSI_MINUS => "minus",
        ANSI_EQUAL => "equal",
        ANSI_LEFT_BRACKET => "leftbracket",
        ANSI_RIGHT_BRACKET => "rightbracket",
        ANSI_BACKSLASH => "backslash",
        ANSI_SEMICOLON => "semicolon",
        ANSI_QUOTE => "quote",
        ANSI_COMMA => "comma",
        ANSI_PERIOD => "period",
        ANSI_SLASH => "slash",
        ANSI_GRAVE => "grave",
        ISO_SECTION => "section",
        _ => "unknown",
    }
}

fn named_to_keycode(named: NamedKey) -> Option<u16> {
    Some(match named {
        NamedKey::Space => SPACE,
        NamedKey::Enter => RETURN,
        NamedKey::Escape => ESCAPE,
        NamedKey::Tab => TAB,
        NamedKey::Backspace => DELETE,
        NamedKey::Delete => FORWARD_DELETE,
        NamedKey::Home => HOME,
        NamedKey::End => END,
        NamedKey::PageUp => PAGE_UP,
        NamedKey::PageDown => PAGE_DOWN,
        NamedKey::Up => UP_ARROW,
        NamedKey::Down => DOWN_ARROW,
        NamedKey::Left => LEFT_ARROW,
        NamedKey::Right => RIGHT_ARROW,
        NamedKey::Insert | NamedKey::PrintScreen | NamedKey::Pause => return None,
    })
}

const LETTERS: [u16; 26] = [
    ANSI_A, ANSI_B, ANSI_C, ANSI_D, ANSI_E, ANSI_F, ANSI_G, ANSI_H, ANSI_I, ANSI_J, ANSI_K, ANSI_L,
    ANSI_M, ANSI_N, ANSI_O, ANSI_P, ANSI_Q, ANSI_R, ANSI_S, ANSI_T, ANSI_U, ANSI_V, ANSI_W, ANSI_X,
    ANSI_Y, ANSI_Z,
];

const DIGITS: [u16; 10] = [
    ANSI_0, ANSI_1, ANSI_2, ANSI_3, ANSI_4, ANSI_5, ANSI_6, ANSI_7, ANSI_8, ANSI_9,
];

const FUNCTION_KEYS: [u16; 12] = [F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalLayout {
    Ansi,
    Iso,
    Jis,
}

impl PhysicalLayout {
    pub fn from_layout_type(kind: u32) -> Self {
        match kind {
            0x4953_4F20 => Self::Iso,
            0x4A49_5320 => Self::Jis,
            _ => Self::Ansi,
        }
    }
}

const ISO_SWAP: [(u16, u16); 2] = [(0x35, ISO_SECTION), (0x64, ANSI_GRAVE)];

#[rustfmt::skip]
const HID_TO_KEYCODE: &[(u16, u16)] = &[
    (0x04, ANSI_A), (0x05, ANSI_B), (0x06, ANSI_C), (0x07, ANSI_D),
    (0x08, ANSI_E), (0x09, ANSI_F), (0x0A, ANSI_G), (0x0B, ANSI_H),
    (0x0C, ANSI_I), (0x0D, ANSI_J), (0x0E, ANSI_K), (0x0F, ANSI_L),
    (0x10, ANSI_M), (0x11, ANSI_N), (0x12, ANSI_O), (0x13, ANSI_P),
    (0x14, ANSI_Q), (0x15, ANSI_R), (0x16, ANSI_S), (0x17, ANSI_T),
    (0x18, ANSI_U), (0x19, ANSI_V), (0x1A, ANSI_W), (0x1B, ANSI_X),
    (0x1C, ANSI_Y), (0x1D, ANSI_Z),
    (0x1E, ANSI_1), (0x1F, ANSI_2), (0x20, ANSI_3), (0x21, ANSI_4),
    (0x22, ANSI_5), (0x23, ANSI_6), (0x24, ANSI_7), (0x25, ANSI_8),
    (0x26, ANSI_9), (0x27, ANSI_0),
    (0x28, RETURN), (0x29, ESCAPE), (0x2A, DELETE), (0x2B, TAB), (0x2C, SPACE),
    (0x2D, ANSI_MINUS), (0x2E, ANSI_EQUAL), (0x2F, ANSI_LEFT_BRACKET),
    (0x30, ANSI_RIGHT_BRACKET), (0x31, ANSI_BACKSLASH), (0x32, ANSI_BACKSLASH),
    (0x33, ANSI_SEMICOLON), (0x34, ANSI_QUOTE), (0x35, ANSI_GRAVE),
    (0x36, ANSI_COMMA), (0x37, ANSI_PERIOD), (0x38, ANSI_SLASH),
    (0x39, 0x39),
    (0x3A, F1), (0x3B, F2), (0x3C, F3), (0x3D, F4), (0x3E, F5), (0x3F, F6),
    (0x40, F7), (0x41, F8), (0x42, F9), (0x43, F10), (0x44, F11), (0x45, F12),
    (0x68, 0x69), (0x69, 0x6B), (0x6A, 0x71),
    (0x6B, 0x6A), (0x6C, 0x40), (0x6D, 0x4F), (0x6E, 0x50), (0x6F, 0x5A),
    (0x46, 0x69), (0x47, 0x6B), (0x48, 0x71),
    (0x49, 0x72), (0x4A, HOME), (0x4B, PAGE_UP), (0x4C, FORWARD_DELETE),
    (0x4D, END), (0x4E, PAGE_DOWN),
    (0x4F, RIGHT_ARROW), (0x50, LEFT_ARROW), (0x51, DOWN_ARROW), (0x52, UP_ARROW),
    (0x53, 0x47), (0x54, 0x4B), (0x55, 0x43), (0x56, 0x4E), (0x57, 0x45),
    (0x58, 0x4C), (0x59, 0x53), (0x5A, 0x54), (0x5B, 0x55), (0x5C, 0x56),
    (0x5D, 0x57), (0x5E, 0x58), (0x5F, 0x59), (0x60, 0x5B), (0x61, 0x5C),
    (0x62, 0x52), (0x63, 0x41), (0x64, ISO_SECTION), (0x65, 0x6E), (0x67, 0x51),
    (0x7F, 0x4A), (0x80, 0x48), (0x81, 0x49),
    (0x87, 0x5E), (0x89, 0x5D), (0x90, 0x68), (0x91, 0x66),
    (0xE0, 0x3B), (0xE1, 0x38), (0xE2, 0x3A), (0xE3, 0x37),
    (0xE4, 0x3E), (0xE5, 0x3C), (0xE6, 0x3D), (0xE7, 0x36),
];

pub fn from_hid_usage(usage: u16, layout: PhysicalLayout) -> Option<u16> {
    if layout == PhysicalLayout::Iso {
        if let Some(&(_, code)) = ISO_SWAP.iter().find(|(from, _)| *from == usage) {
            return Some(code);
        }
    }
    HID_TO_KEYCODE
        .iter()
        .find(|(from, _)| *from == usage)
        .map(|&(_, code)| code)
}

pub fn to_hid_usage(code: u16, layout: PhysicalLayout) -> Option<u16> {
    if layout == PhysicalLayout::Iso {
        if let Some(&(usage, _)) = ISO_SWAP.iter().find(|(_, to)| *to == code) {
            return Some(usage);
        }
    }
    HID_TO_KEYCODE
        .iter()
        .find(|(_, to)| *to == code)
        .map(|&(usage, _)| usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grammar;

    fn code(input: &str) -> Option<u16> {
        key_to_keycode(grammar::parse(input).unwrap().key)
    }

    #[test]
    fn maps_grammar_keys_to_macos_keycodes() {
        assert_eq!(code("r"), Some(ANSI_R));
        assert_eq!(code("0"), Some(ANSI_0));
        assert_eq!(code("f12"), Some(F12));
        assert_eq!(code("space"), Some(SPACE));
        assert_eq!(code("backspace"), Some(DELETE));
        assert_eq!(code("delete"), Some(FORWARD_DELETE));
        assert_eq!(code("left"), Some(LEFT_ARROW));
    }

    #[test]
    fn rejects_macos_unsupported_named_keys() {
        assert_eq!(code("insert"), None);
        assert_eq!(code("printscreen"), None);
        assert_eq!(code("pause"), None);
    }

    #[test]
    fn rejects_out_of_range_public_key_values() {
        assert_eq!(key_to_keycode(Key::Letter(26)), None);
        assert_eq!(key_to_keycode(Key::Digit(10)), None);
        assert_eq!(key_to_keycode(Key::Function(13)), None);
    }

    #[test]
    fn parses_keyremap_key_names() {
        assert_eq!(parse_key("A"), Some(ANSI_A));
        assert_eq!(parse_key("enter"), Some(RETURN));
        assert_eq!(parse_key("backspace"), Some(DELETE));
        assert_eq!(parse_key("delete"), Some(FORWARD_DELETE));
        assert_eq!(parse_key("del"), Some(FORWARD_DELETE));
        assert_eq!(parse_key("forwarddelete"), Some(FORWARD_DELETE));
        assert_eq!(parse_key("minus"), Some(ANSI_MINUS));
        assert_eq!(parse_key("<"), Some(ISO_SECTION));
    }

    #[test]
    fn formats_keyremap_key_names() {
        assert_eq!(key_name(ANSI_A), "a");
        assert_eq!(key_name(RETURN), "return");
        assert_eq!(key_name(DELETE), "backspace");
        assert_eq!(key_name(FORWARD_DELETE), "delete");
        assert_eq!(key_name(ANSI_MINUS), "minus");
        assert_eq!(key_name(ISO_SECTION), "section");
        assert_eq!(key_name(u16::MAX), "unknown");
    }

    const LAYOUTS: [PhysicalLayout; 3] = [
        PhysicalLayout::Ansi,
        PhysicalLayout::Iso,
        PhysicalLayout::Jis,
    ];

    #[test]
    fn every_named_keycode_survives_a_trip_through_hid() {
        for layout in LAYOUTS {
            for code in 0..0x80u16 {
                if key_name(code) == "unknown" {
                    continue;
                }
                let usage = to_hid_usage(code, layout)
                    .unwrap_or_else(|| panic!("{} has no HID usage", key_name(code)));
                assert_eq!(
                    from_hid_usage(usage, layout),
                    Some(code),
                    "{layout:?} {code:#04x}"
                );
            }
        }
    }

    #[test]
    fn iso_keyboards_swap_the_grave_and_section_keys() {
        assert_eq!(from_hid_usage(0x35, PhysicalLayout::Ansi), Some(ANSI_GRAVE));
        assert_eq!(
            from_hid_usage(0x64, PhysicalLayout::Ansi),
            Some(ISO_SECTION)
        );
        assert_eq!(from_hid_usage(0x35, PhysicalLayout::Iso), Some(ISO_SECTION));
        assert_eq!(from_hid_usage(0x64, PhysicalLayout::Iso), Some(ANSI_GRAVE));
        assert_eq!(to_hid_usage(ISO_SECTION, PhysicalLayout::Iso), Some(0x35));
        assert_eq!(to_hid_usage(ANSI_GRAVE, PhysicalLayout::Iso), Some(0x64));
    }

    #[test]
    fn duplicate_usages_map_back_to_their_canonical_usage() {
        assert_eq!(from_hid_usage(0x46, PhysicalLayout::Ansi), Some(0x69));
        assert_eq!(to_hid_usage(0x69, PhysicalLayout::Ansi), Some(0x68));
        assert_eq!(
            from_hid_usage(0x32, PhysicalLayout::Ansi),
            Some(ANSI_BACKSLASH)
        );
        assert_eq!(
            to_hid_usage(ANSI_BACKSLASH, PhysicalLayout::Ansi),
            Some(0x31)
        );
    }

    #[test]
    fn modifiers_and_caps_lock_have_usages() {
        assert_eq!(from_hid_usage(0xE0, PhysicalLayout::Ansi), Some(0x3B));
        assert_eq!(from_hid_usage(0xE6, PhysicalLayout::Ansi), Some(0x3D));
        assert_eq!(from_hid_usage(0x39, PhysicalLayout::Ansi), Some(0x39));
        assert_eq!(from_hid_usage(0x00, PhysicalLayout::Ansi), None);
    }

    #[test]
    fn layout_type_fourccs_pick_the_physical_layout() {
        assert_eq!(
            PhysicalLayout::from_layout_type(0x4953_4F20),
            PhysicalLayout::Iso
        );
        assert_eq!(
            PhysicalLayout::from_layout_type(0x4A49_5320),
            PhysicalLayout::Jis
        );
        assert_eq!(
            PhysicalLayout::from_layout_type(0x414E_5349),
            PhysicalLayout::Ansi
        );
        assert_eq!(PhysicalLayout::from_layout_type(0), PhysicalLayout::Ansi);
    }
}
