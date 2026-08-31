use qol_hotkeys::evdev;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::rust_connection::RustConnection;

use crate::hotkeys::capture::Combo;

pub(in crate::hotkeys) const POLL_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(8);
pub(in crate::hotkeys) const POLL_WHILE_IDLE: bool = false;

pub(super) fn release_active_grab(
    manager: &global_hotkey::GlobalHotKeyManager,
) -> anyhow::Result<()> {
    manager.release_active_grab()?;
    Ok(())
}

const X11_EVDEV_OFFSET: u16 = 8;

pub(in crate::hotkeys) struct PhysicalHotkeyState {
    connection: RustConnection,
}

pub(in crate::hotkeys) struct PhysicalHotkeySnapshot {
    keys: [u8; 32],
}

impl PhysicalHotkeyState {
    pub(in crate::hotkeys) fn connect() -> Result<Self, String> {
        x11rb::connect(None)
            .map(|(connection, _)| Self { connection })
            .map_err(|error| error.to_string())
    }

    pub(in crate::hotkeys) fn snapshot(&self) -> Result<PhysicalHotkeySnapshot, String> {
        self.connection
            .query_keymap()
            .map_err(|error| error.to_string())?
            .reply()
            .map(|reply| PhysicalHotkeySnapshot { keys: reply.keys })
            .map_err(|error| error.to_string())
    }
}

impl PhysicalHotkeySnapshot {
    pub(in crate::hotkeys) fn supports_reconciliation(&self) -> bool {
        true
    }

    pub(in crate::hotkeys) fn chord_is_pressed(&self, combo: &Combo) -> bool {
        let Some(keycode) = evdev::key_to_keycode(combo.key) else {
            return false;
        };
        if !self.evdev_key_is_pressed(keycode) {
            return false;
        }
        combo.mods.iter().all(|modifier| {
            evdev::modifier_keycodes(*modifier)
                .into_iter()
                .any(|keycode| self.evdev_key_is_pressed(keycode))
        })
    }

    pub(in crate::hotkeys) fn trace_summary(&self) -> String {
        let pressed = [
            (
                "ctrl",
                self.any_key_is_pressed(&[evdev::KEY_LEFTCTRL, evdev::KEY_RIGHTCTRL]),
            ),
            (
                "shift",
                self.any_key_is_pressed(&[evdev::KEY_LEFTSHIFT, evdev::KEY_RIGHTSHIFT]),
            ),
            (
                "super",
                self.any_key_is_pressed(&[evdev::KEY_LEFTMETA, evdev::KEY_RIGHTMETA]),
            ),
            ("left", self.evdev_key_is_pressed(evdev::KEY_LEFT)),
            ("right", self.evdev_key_is_pressed(evdev::KEY_RIGHT)),
            ("up", self.evdev_key_is_pressed(evdev::KEY_UP)),
            ("down", self.evdev_key_is_pressed(evdev::KEY_DOWN)),
        ]
        .into_iter()
        .filter_map(|(name, is_pressed)| is_pressed.then_some(name))
        .collect::<Vec<_>>();
        if pressed.is_empty() {
            "none".into()
        } else {
            pressed.join("+")
        }
    }

    fn any_key_is_pressed(&self, keycodes: &[u16]) -> bool {
        keycodes
            .iter()
            .any(|keycode| self.evdev_key_is_pressed(*keycode))
    }

    #[cfg(feature = "linux_evdev")]
    pub(in crate::hotkeys) fn down_evdev_codes(&self) -> std::collections::HashSet<u16> {
        let mut down = std::collections::HashSet::new();
        for x11_keycode in 0u8..=u8::MAX {
            let byte = usize::from(x11_keycode / 8);
            let bit = x11_keycode % 8;
            if self.keys[byte] & (1 << bit) == 0 {
                continue;
            }
            if let Some(code) = u16::from(x11_keycode).checked_sub(X11_EVDEV_OFFSET) {
                down.insert(code);
            }
        }
        down
    }

    fn evdev_key_is_pressed(&self, keycode: u16) -> bool {
        let Some(x11_keycode) = keycode
            .checked_add(X11_EVDEV_OFFSET)
            .and_then(|keycode| u8::try_from(keycode).ok())
        else {
            return false;
        };
        let byte = usize::from(x11_keycode / 8);
        let bit = x11_keycode % 8;
        self.keys[byte] & (1 << bit) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(pressed_evdev_keycodes: &[u16]) -> PhysicalHotkeySnapshot {
        let mut snapshot = PhysicalHotkeySnapshot { keys: [0; 32] };
        for keycode in pressed_evdev_keycodes {
            let x11_keycode = u8::try_from(keycode + X11_EVDEV_OFFSET).unwrap();
            snapshot.keys[usize::from(x11_keycode / 8)] |= 1 << (x11_keycode % 8);
        }
        snapshot
    }

    #[cfg(feature = "linux_evdev")]
    #[test]
    fn down_evdev_codes_inverts_the_x11_offset() {
        let down = snapshot(&[evdev::KEY_DOWN, evdev::KEY_LEFTSHIFT]).down_evdev_codes();
        assert!(down.contains(&evdev::KEY_DOWN));
        assert!(down.contains(&evdev::KEY_LEFTSHIFT));
        assert_eq!(down.len(), 2);
    }

    #[test]
    fn requires_terminal_key_and_every_bound_modifier() {
        let combo = crate::hotkeys::capture::parse_combo("Ctrl+Shift+Super+Left").unwrap();
        let chord = [
            evdev::KEY_LEFTCTRL,
            evdev::KEY_LEFTSHIFT,
            evdev::KEY_LEFTMETA,
            evdev::KEY_LEFT,
        ];
        assert!(snapshot(&chord).chord_is_pressed(&combo));

        for released in chord {
            let pressed = chord
                .into_iter()
                .filter(|keycode| *keycode != released)
                .collect::<Vec<_>>();
            assert!(
                !snapshot(&pressed).chord_is_pressed(&combo),
                "released evdev keycode {released}"
            );
        }
    }

    #[test]
    fn accepts_either_side_of_each_modifier() {
        let combo = crate::hotkeys::capture::parse_combo("Ctrl+Shift+Super+Right").unwrap();
        let chord = [
            evdev::KEY_RIGHTCTRL,
            evdev::KEY_RIGHTSHIFT,
            evdev::KEY_RIGHTMETA,
            evdev::KEY_RIGHT,
        ];
        assert!(snapshot(&chord).chord_is_pressed(&combo));
    }

    #[test]
    fn trace_summary_only_reports_relevant_modifier_and_direction_keys() {
        let state = snapshot(&[
            evdev::KEY_LEFTCTRL,
            evdev::KEY_RIGHTSHIFT,
            evdev::KEY_LEFTMETA,
            evdev::KEY_LEFT,
            evdev::KEY_DOWN,
            evdev::KEY_SPACE,
        ]);

        assert_eq!(state.trace_summary(), "ctrl+shift+super+left+down");
    }
}
