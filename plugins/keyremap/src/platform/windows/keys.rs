use qol_hotkeys::macos_keycode as keycode;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_0, VK_1, VK_2, VK_3, VK_4, VK_5, VK_6, VK_7, VK_8, VK_9, VK_A, VK_APPS, VK_B,
    VK_BACK, VK_C, VK_CAPITAL, VK_D, VK_DELETE, VK_DIVIDE, VK_DOWN, VK_E, VK_END, VK_ESCAPE, VK_F,
    VK_F1, VK_F10, VK_F11, VK_F12, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9, VK_G,
    VK_H, VK_HOME, VK_I, VK_INSERT, VK_J, VK_K, VK_L, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT,
    VK_LWIN, VK_M, VK_N, VK_NEXT, VK_NUMLOCK, VK_O, VK_OEM_1, VK_OEM_102, VK_OEM_2, VK_OEM_3,
    VK_OEM_4, VK_OEM_5, VK_OEM_6, VK_OEM_7, VK_OEM_COMMA, VK_OEM_MINUS, VK_OEM_PERIOD, VK_OEM_PLUS,
    VK_P, VK_PRIOR, VK_Q, VK_R, VK_RCONTROL, VK_RETURN, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_RWIN,
    VK_S, VK_SNAPSHOT, VK_SPACE, VK_T, VK_TAB, VK_U, VK_UP, VK_V, VK_W, VK_X, VK_Y, VK_Z,
};

use crate::platform::engine::config::{ModifierTarget, RemapConfig};

pub(super) const UNMAPPED: u16 = u16::MAX;
pub(super) const MASK_KEY: VIRTUAL_KEY = 0xE8;
pub(super) const MENU_BITS: u8 = 0x04 | 0x08 | 0x40 | 0x80;

pub(super) const MODIFIERS: [(VIRTUAL_KEY, u8); 8] = [
    (VK_LCONTROL, 0x01),
    (VK_LSHIFT, 0x02),
    (VK_LMENU, 0x04),
    (VK_LWIN, 0x08),
    (VK_RCONTROL, 0x10),
    (VK_RSHIFT, 0x20),
    (VK_RMENU, 0x40),
    (VK_RWIN, 0x80),
];

const KEYS: [(u16, VIRTUAL_KEY); 74] = [
    (keycode::ANSI_A, VK_A),
    (keycode::ANSI_B, VK_B),
    (keycode::ANSI_C, VK_C),
    (keycode::ANSI_D, VK_D),
    (keycode::ANSI_E, VK_E),
    (keycode::ANSI_F, VK_F),
    (keycode::ANSI_G, VK_G),
    (keycode::ANSI_H, VK_H),
    (keycode::ANSI_I, VK_I),
    (keycode::ANSI_J, VK_J),
    (keycode::ANSI_K, VK_K),
    (keycode::ANSI_L, VK_L),
    (keycode::ANSI_M, VK_M),
    (keycode::ANSI_N, VK_N),
    (keycode::ANSI_O, VK_O),
    (keycode::ANSI_P, VK_P),
    (keycode::ANSI_Q, VK_Q),
    (keycode::ANSI_R, VK_R),
    (keycode::ANSI_S, VK_S),
    (keycode::ANSI_T, VK_T),
    (keycode::ANSI_U, VK_U),
    (keycode::ANSI_V, VK_V),
    (keycode::ANSI_W, VK_W),
    (keycode::ANSI_X, VK_X),
    (keycode::ANSI_Y, VK_Y),
    (keycode::ANSI_Z, VK_Z),
    (keycode::ANSI_0, VK_0),
    (keycode::ANSI_1, VK_1),
    (keycode::ANSI_2, VK_2),
    (keycode::ANSI_3, VK_3),
    (keycode::ANSI_4, VK_4),
    (keycode::ANSI_5, VK_5),
    (keycode::ANSI_6, VK_6),
    (keycode::ANSI_7, VK_7),
    (keycode::ANSI_8, VK_8),
    (keycode::ANSI_9, VK_9),
    (keycode::RETURN, VK_RETURN),
    (keycode::TAB, VK_TAB),
    (keycode::SPACE, VK_SPACE),
    (keycode::DELETE, VK_BACK),
    (keycode::FORWARD_DELETE, VK_DELETE),
    (keycode::ESCAPE, VK_ESCAPE),
    (keycode::LEFT_ARROW, VK_LEFT),
    (keycode::RIGHT_ARROW, VK_RIGHT),
    (keycode::DOWN_ARROW, VK_DOWN),
    (keycode::UP_ARROW, VK_UP),
    (keycode::HOME, VK_HOME),
    (keycode::END, VK_END),
    (keycode::PAGE_UP, VK_PRIOR),
    (keycode::PAGE_DOWN, VK_NEXT),
    (keycode::F1, VK_F1),
    (keycode::F2, VK_F2),
    (keycode::F3, VK_F3),
    (keycode::F4, VK_F4),
    (keycode::F5, VK_F5),
    (keycode::F6, VK_F6),
    (keycode::F7, VK_F7),
    (keycode::F8, VK_F8),
    (keycode::F9, VK_F9),
    (keycode::F10, VK_F10),
    (keycode::F11, VK_F11),
    (keycode::F12, VK_F12),
    (keycode::ANSI_MINUS, VK_OEM_MINUS),
    (keycode::ANSI_EQUAL, VK_OEM_PLUS),
    (keycode::ANSI_LEFT_BRACKET, VK_OEM_4),
    (keycode::ANSI_RIGHT_BRACKET, VK_OEM_6),
    (keycode::ANSI_BACKSLASH, VK_OEM_5),
    (keycode::ANSI_SEMICOLON, VK_OEM_1),
    (keycode::ANSI_QUOTE, VK_OEM_7),
    (keycode::ANSI_COMMA, VK_OEM_COMMA),
    (keycode::ANSI_PERIOD, VK_OEM_PERIOD),
    (keycode::ANSI_SLASH, VK_OEM_2),
    (keycode::ANSI_GRAVE, VK_OEM_3),
    (keycode::ISO_SECTION, VK_OEM_102),
];

const EXTENDED: [VIRTUAL_KEY; 18] = [
    VK_RCONTROL,
    VK_RMENU,
    VK_LWIN,
    VK_RWIN,
    VK_APPS,
    VK_INSERT,
    VK_DELETE,
    VK_HOME,
    VK_END,
    VK_PRIOR,
    VK_NEXT,
    VK_LEFT,
    VK_RIGHT,
    VK_UP,
    VK_DOWN,
    VK_DIVIDE,
    VK_NUMLOCK,
    VK_SNAPSHOT,
];

pub(super) fn canonical(vk: VIRTUAL_KEY) -> Option<u16> {
    KEYS.iter()
        .find(|(_, key)| *key == vk)
        .map(|(code, _)| *code)
}

pub(super) fn virtual_key(code: u16) -> Option<VIRTUAL_KEY> {
    KEYS.iter()
        .find(|(known, _)| *known == code)
        .map(|(_, key)| *key)
}

pub(super) fn modifier_bit(vk: VIRTUAL_KEY) -> Option<u8> {
    MODIFIERS
        .iter()
        .find(|(key, _)| *key == vk)
        .map(|(_, bit)| *bit)
}

pub(super) fn is_extended(vk: VIRTUAL_KEY) -> bool {
    EXTENDED.contains(&vk)
}

pub(super) fn modifier_source(vk: VIRTUAL_KEY) -> Option<(ModifierTarget, bool)> {
    match vk {
        VK_CAPITAL => Some((ModifierTarget::CapsLock, false)),
        VK_LCONTROL => Some((ModifierTarget::Control, false)),
        VK_RCONTROL => Some((ModifierTarget::Control, true)),
        VK_LMENU => Some((ModifierTarget::Option, false)),
        VK_RMENU => Some((ModifierTarget::Option, true)),
        VK_LWIN => Some((ModifierTarget::Command, false)),
        VK_RWIN => Some((ModifierTarget::Command, true)),
        _ => None,
    }
}

pub(super) fn target_key(
    target: ModifierTarget,
    right: bool,
    source: VIRTUAL_KEY,
) -> Option<VIRTUAL_KEY> {
    let side = |left, right_key| if right { right_key } else { left };
    match target {
        ModifierTarget::CapsLock => Some(VK_CAPITAL),
        ModifierTarget::Control => Some(side(VK_LCONTROL, VK_RCONTROL)),
        ModifierTarget::Option => Some(side(VK_LMENU, VK_RMENU)),
        ModifierTarget::Command => Some(side(VK_LWIN, VK_RWIN)),
        ModifierTarget::Escape => Some(VK_ESCAPE),
        ModifierTarget::Fn => Some(source),
        ModifierTarget::None => None,
    }
}

pub(super) fn modifier_key_issues(config: &RemapConfig) -> Vec<String> {
    let keys = config.modifier_keys;
    let mut issues = Vec::new();
    if keys.fn_key != ModifierTarget::Fn {
        issues.push(
            "modifier_keys.fn has no effect on Windows, which never sees the Globe / fn key"
                .to_string(),
        );
    }
    for (name, target) in [
        ("caps_lock", keys.caps_lock),
        ("control", keys.control),
        ("option", keys.option),
        ("command", keys.command),
    ] {
        if target == ModifierTarget::Fn {
            issues.push(format!(
                "modifier_keys.{name} cannot act as fn on Windows, which has no fn key to send"
            ));
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_key_names_translate_to_windows_virtual_keys() {
        let cases = [
            ("a", VK_A),
            ("z", VK_Z),
            ("0", VK_0),
            ("9", VK_9),
            ("return", VK_RETURN),
            ("tab", VK_TAB),
            ("backspace", VK_BACK),
            ("delete", VK_DELETE),
            ("escape", VK_ESCAPE),
            ("left", VK_LEFT),
            ("pageup", VK_PRIOR),
            ("pagedown", VK_NEXT),
            ("f1", VK_F1),
            ("f12", VK_F12),
            ("-", VK_OEM_MINUS),
            ("=", VK_OEM_PLUS),
            ("[", VK_OEM_4),
            ("\\", VK_OEM_5),
            (";", VK_OEM_1),
            ("/", VK_OEM_2),
            ("`", VK_OEM_3),
            ("<", VK_OEM_102),
        ];
        for (name, vk) in cases {
            let code = keycode::parse_key(name).unwrap();
            assert_eq!(virtual_key(code), Some(vk), "{name}");
            assert_eq!(canonical(vk), Some(code), "{name}");
        }
    }

    #[test]
    fn the_key_table_is_one_to_one() {
        for (index, (code, vk)) in KEYS.iter().enumerate() {
            assert!(
                KEYS[index + 1..]
                    .iter()
                    .all(|(other_code, other_vk)| other_code != code && other_vk != vk),
                "{code:#x} {vk:#x}"
            );
        }
    }

    #[test]
    fn modifiers_and_keys_without_a_rule_name_stay_unmapped() {
        for vk in [
            VK_LSHIFT, VK_RWIN, VK_CAPITAL, VK_NUMLOCK, VK_APPS, MASK_KEY,
        ] {
            assert_eq!(canonical(vk), None, "{vk:#x}");
        }
        assert_eq!(virtual_key(UNMAPPED), None);
    }

    #[test]
    fn modifier_targets_keep_the_side_of_the_source_key() {
        let cases = [
            (
                ModifierTarget::Control,
                false,
                VK_CAPITAL,
                Some(VK_LCONTROL),
            ),
            (ModifierTarget::Control, true, VK_RMENU, Some(VK_RCONTROL)),
            (ModifierTarget::Option, false, VK_LWIN, Some(VK_LMENU)),
            (ModifierTarget::Command, true, VK_RCONTROL, Some(VK_RWIN)),
            (
                ModifierTarget::CapsLock,
                true,
                VK_RCONTROL,
                Some(VK_CAPITAL),
            ),
            (ModifierTarget::Escape, false, VK_CAPITAL, Some(VK_ESCAPE)),
            (ModifierTarget::Fn, false, VK_LMENU, Some(VK_LMENU)),
            (ModifierTarget::None, false, VK_CAPITAL, None),
        ];
        for (target, right, source, expected) in cases {
            assert_eq!(
                target_key(target, right, source),
                expected,
                "{target:?} right={right}"
            );
        }
    }

    #[test]
    fn modifier_sources_cover_both_sides_of_every_modifier() {
        let cases = [
            (VK_CAPITAL, Some((ModifierTarget::CapsLock, false))),
            (VK_LCONTROL, Some((ModifierTarget::Control, false))),
            (VK_RCONTROL, Some((ModifierTarget::Control, true))),
            (VK_LMENU, Some((ModifierTarget::Option, false))),
            (VK_RMENU, Some((ModifierTarget::Option, true))),
            (VK_LWIN, Some((ModifierTarget::Command, false))),
            (VK_RWIN, Some((ModifierTarget::Command, true))),
            (VK_LSHIFT, None),
            (VK_A, None),
        ];
        for (vk, expected) in cases {
            assert_eq!(modifier_source(vk), expected, "{vk:#x}");
        }
    }

    #[test]
    fn fn_settings_are_reported_as_issues() {
        let cases = [
            (serde_json::json!({}), 0),
            (
                serde_json::json!({ "modifier_keys": { "caps_lock": "control" } }),
                0,
            ),
            (
                serde_json::json!({ "modifier_keys": { "fn": "control" } }),
                1,
            ),
            (
                serde_json::json!({ "modifier_keys": { "caps_lock": "fn", "command": "fn" } }),
                2,
            ),
        ];
        for (raw, expected) in cases {
            let config: RemapConfig = serde_json::from_value(raw.clone()).unwrap();
            assert_eq!(modifier_key_issues(&config).len(), expected, "{raw}");
        }
    }
}
