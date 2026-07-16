use qol_hotkeys::evdev;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::rust_connection::RustConnection;

use crate::hotkeys::capture::parse_combo;

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
    pub(in crate::hotkeys) fn chord_is_pressed(&self, raw_key: &str) -> bool {
        let Some(combo) = parse_combo(raw_key) else {
            return false;
        };
        if !self.evdev_key_is_pressed(combo.key) {
            return false;
        }
        combo.mods.iter().all(|modifier| {
            evdev::modifier_keycodes(*modifier)
                .into_iter()
                .any(|keycode| self.evdev_key_is_pressed(keycode))
        })
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

    #[test]
    fn requires_terminal_key_and_every_bound_modifier() {
        let chord = [
            evdev::KEY_LEFTCTRL,
            evdev::KEY_LEFTSHIFT,
            evdev::KEY_LEFTMETA,
            evdev::KEY_LEFT,
        ];
        assert!(snapshot(&chord).chord_is_pressed("Ctrl+Shift+Super+Left"));

        for released in chord {
            let pressed = chord
                .into_iter()
                .filter(|keycode| *keycode != released)
                .collect::<Vec<_>>();
            assert!(
                !snapshot(&pressed).chord_is_pressed("Ctrl+Shift+Super+Left"),
                "released evdev keycode {released}"
            );
        }
    }

    #[test]
    fn accepts_either_side_of_each_modifier() {
        let chord = [
            evdev::KEY_RIGHTCTRL,
            evdev::KEY_RIGHTSHIFT,
            evdev::KEY_RIGHTMETA,
            evdev::KEY_RIGHT,
        ];
        assert!(snapshot(&chord).chord_is_pressed("Ctrl+Shift+Super+Right"));
    }
}
