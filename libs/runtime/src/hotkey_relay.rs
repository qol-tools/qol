use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const SOCKET_FILE: &str = "qol-hotkey-relay.sock";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyModifier {
    Shift,
    Ctrl,
    Alt,
    Super,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HotkeyCombo {
    #[serde(default)]
    pub mods: Vec<HotkeyModifier>,
    pub mac_keycode: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyPress {
    pub combo: HotkeyCombo,
    pub pressed: bool,
}

pub fn socket_path() -> Option<PathBuf> {
    crate::plugin_socket::assigned_socket_path(SOCKET_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_press_reads_back_from_the_json_a_relay_sends() {
        let press = HotkeyPress {
            combo: HotkeyCombo {
                mods: vec![HotkeyModifier::Alt],
                mac_keycode: 48,
            },
            pressed: true,
        };
        let wire = serde_json::to_string(&press).unwrap();
        assert_eq!(
            wire,
            r#"{"combo":{"mods":["alt"],"mac_keycode":48},"pressed":true}"#
        );
        assert_eq!(serde_json::from_str::<HotkeyPress>(&wire).unwrap(), press);
    }
}
