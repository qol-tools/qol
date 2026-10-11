use super::super::hook_rules;
use super::super::key_matcher::{KeyCombo, KeyTransition};
use super::super::recorder_hub::{Decision, RecorderHub};
use qol_hotkeys::grammar::{self, Hotkey};
use qol_hotkeys::windows_keycode;
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyExW, MAPVK_VK_TO_CHAR};

const VK_ESCAPE: u16 = 0x1B;
const MODIFIER_KEYS: [u16; 11] = [
    0x10, 0x11, 0x12, 0x5B, 0x5C, 0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5,
];
const DEAD_KEY_FLAG: u32 = 0x8000_0000;

thread_local! {
    static CLAIMED: RefCell<HashSet<u16>> = RefCell::default();
}

pub(super) fn global() -> &'static RecorderHub {
    static RECORDER: RecorderHub = RecorderHub::new("llhook", hook_armed);
    &RECORDER
}

pub(super) fn claimed(key: u16) -> bool {
    CLAIMED.with_borrow(|claimed| claimed.contains(&key))
}

pub(super) fn record(transition: KeyTransition, observed: &KeyCombo) -> bool {
    if claimed(observed.key) {
        if transition == KeyTransition::Release {
            CLAIMED.with_borrow_mut(|claimed| claimed.remove(&observed.key));
        }
        return true;
    }
    if transition != KeyTransition::Press
        || MODIFIER_KEYS.contains(&observed.key)
        || !global().recording()
    {
        return false;
    }
    CLAIMED.with_borrow_mut(|claimed| claimed.insert(observed.key));
    if hook_rules::needs_menu_mask(transition, true, &observed.mods) {
        super::send_menu_mask();
    }
    global().settle(decide(observed, symbol_at));
    true
}

fn hook_armed() -> bool {
    super::HOOK_ARMED.load(Ordering::SeqCst)
}

fn decide(observed: &KeyCombo, symbol_at: impl Fn(u16) -> Option<char>) -> Decision {
    if observed.key == VK_ESCAPE {
        return Decision::Cancel;
    }
    let Some(key) = windows_keycode::vk_to_key(observed.key)
        .or_else(|| symbol_at(observed.key).and_then(grammar::symbol_key))
    else {
        return Decision::Ignore;
    };
    match grammar::format(&Hotkey {
        mods: observed.mods.clone(),
        key,
    }) {
        Some(formatted) => Decision::Complete(formatted),
        None => Decision::Ignore,
    }
}

fn symbol_at(key: u16) -> Option<char> {
    let mapped =
        unsafe { MapVirtualKeyExW(u32::from(key), MAPVK_VK_TO_CHAR, super::foreground_layout()) };
    char::from_u32(mapped & !DEAD_KEY_FLAG).filter(|symbol| *symbol != '\0')
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_hotkeys::grammar::Modifier;

    fn combo(mods: &[Modifier], key: u16) -> KeyCombo {
        KeyCombo {
            mods: mods.iter().copied().collect(),
            key,
        }
    }

    fn danish(key: u16) -> Option<char> {
        match key {
            0xBB => Some('+'),
            0xBD => Some('-'),
            0xBC => Some(','),
            0xDE => Some('ø'),
            _ => None,
        }
    }

    #[test]
    fn decides_between_complete_cancel_and_ignore_for_hook_presses() {
        let cases = [
            (combo(&[], 0x52), Decision::Complete("R".into())),
            (
                combo(&[Modifier::Shift, Modifier::Super], 0x52),
                Decision::Complete("Shift+Super+R".into()),
            ),
            (
                combo(&[Modifier::Super], 0x20),
                Decision::Complete("Super+Space".into()),
            ),
            (
                combo(&[Modifier::Ctrl, Modifier::Alt], 0x4B),
                Decision::Complete("Ctrl+Alt+K".into()),
            ),
            (
                combo(&[Modifier::Ctrl], 0x77),
                Decision::Complete("Ctrl+F8".into()),
            ),
            (
                combo(&[Modifier::Super], 0x25),
                Decision::Complete("Super+Left".into()),
            ),
            (
                combo(&[Modifier::Super], 0x35),
                Decision::Complete("Super+5".into()),
            ),
            (
                combo(&[Modifier::Super], 0xBB),
                Decision::Complete("Super+Plus".into()),
            ),
            (
                combo(&[Modifier::Super], 0xBD),
                Decision::Complete("Super+-".into()),
            ),
            (
                combo(&[Modifier::Alt], 0xDE),
                Decision::Complete("Alt+ø".into()),
            ),
            (combo(&[], VK_ESCAPE), Decision::Cancel),
            (combo(&[Modifier::Ctrl], VK_ESCAPE), Decision::Cancel),
            (combo(&[Modifier::Super], 0x14), Decision::Ignore),
            (combo(&[Modifier::Super], 0x60), Decision::Ignore),
        ];
        for (observed, expected) in cases {
            assert_eq!(
                decide(&observed, danish),
                expected,
                "key: {:#x} mods: {:?}",
                observed.key,
                observed.mods
            );
        }
    }

    #[test]
    fn symbol_recordings_round_trip_through_the_grammar() {
        for (key, expected) in [(0xBB, "Super+Plus"), (0xBC, "Super+,")] {
            let Decision::Complete(formatted) = decide(&combo(&[Modifier::Super], key), danish)
            else {
                panic!("key {key:#x} must record");
            };
            assert_eq!(formatted, expected);
            assert_eq!(
                grammar::parse(&formatted).map(|hotkey| hotkey.key),
                danish(key).and_then(grammar::symbol_key),
                "key: {key:#x}"
            );
        }
    }
}
