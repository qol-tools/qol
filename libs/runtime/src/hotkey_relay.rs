use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub const SOCKET_FILE: &str = "qol-hotkey-relay.sock";
pub const LEASE_TTL: Duration = Duration::from_secs(3);

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyLease {
    pub lease: Vec<HotkeyCombo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RelayMessage {
    Press(HotkeyPress),
    Lease(HotkeyLease),
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
        assert_eq!(
            serde_json::from_str::<RelayMessage>(&wire).unwrap(),
            RelayMessage::Press(press)
        );
    }

    #[test]
    fn a_lease_reads_back_as_a_lease_and_never_as_a_press() {
        let lease = HotkeyLease {
            lease: vec![HotkeyCombo {
                mods: vec![HotkeyModifier::Shift, HotkeyModifier::Super],
                mac_keycode: 15,
            }],
        };
        let wire = serde_json::to_string(&lease).unwrap();
        assert_eq!(
            wire,
            r#"{"lease":[{"mods":["shift","super"],"mac_keycode":15}]}"#
        );
        assert_eq!(
            serde_json::from_str::<RelayMessage>(&wire).unwrap(),
            RelayMessage::Lease(lease)
        );
        assert_eq!(
            serde_json::from_str::<RelayMessage>(r#"{"lease":[]}"#).unwrap(),
            RelayMessage::Lease(HotkeyLease { lease: Vec::new() })
        );
    }
}
