use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use qol_hotkeys::macos_keycode::PhysicalLayout;
use qol_runtime::hotkey_relay::{HotkeyCombo, HotkeyModifier, HotkeyPress};
use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind};
use qol_runtime::PlatformStateClient;
use serde_json::{json, Value};

use super::compile::{Gap, ACTIVE_VARIABLE};
use super::key_codes;

const RESUBSCRIBE: Duration = Duration::from_secs(2);
const SNAPSHOT_WAIT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Hotkeys {
    pub(crate) relay: Option<PathBuf>,
    pub(crate) combos: Vec<HotkeyCombo>,
}

impl Hotkeys {
    fn from_event(event: RuntimeEvent) -> Option<Self> {
        match event {
            RuntimeEvent::HotkeysChanged { hotkeys, relay } => Some(Self {
                relay,
                combos: hotkeys,
            }),
            _ => None,
        }
    }

    pub(super) fn manipulators(&self, physical: PhysicalLayout) -> (Vec<Value>, Vec<Gap>) {
        let Some(relay) = &self.relay else {
            return (Vec::new(), Vec::new());
        };
        let mut manipulators = Vec::new();
        let mut gaps = Vec::new();
        for combo in &self.combos {
            let Some(key_code) = key_codes::for_keycode(combo.mac_keycode, physical) else {
                gaps.push(Gap {
                    rule: format!("qol hotkey {}", label(combo)),
                    reason: format!(
                        "Karabiner has no name for keycode {:#04x}",
                        combo.mac_keycode
                    ),
                });
                continue;
            };
            let mut modifiers = json!({ "optional": ["caps_lock"] });
            if !combo.mods.is_empty() {
                modifiers["mandatory"] = combo.mods.iter().map(|&m| karabiner_mod(m)).collect();
            }
            let send = |pressed: bool| {
                json!([{ "send_user_command": {
                    "endpoint": relay,
                    "payload": HotkeyPress { combo: combo.clone(), pressed },
                } }])
            };
            manipulators.push(json!({
                "type": "basic",
                "from": { "key_code": key_code, "modifiers": modifiers },
                "to": send(true),
                "to_after_key_up": send(false),
                "conditions": [{ "type": "variable_if", "name": ACTIVE_VARIABLE, "value": 1 }],
            }));
        }
        (manipulators, gaps)
    }
}

fn label(combo: &HotkeyCombo) -> String {
    let mut parts: Vec<String> = combo
        .mods
        .iter()
        .map(|&modifier| karabiner_mod(modifier).to_string())
        .collect();
    parts.push(format!("{:#04x}", combo.mac_keycode));
    parts.join("+")
}

fn karabiner_mod(modifier: HotkeyModifier) -> &'static str {
    match modifier {
        HotkeyModifier::Shift => "shift",
        HotkeyModifier::Ctrl => "control",
        HotkeyModifier::Alt => "option",
        HotkeyModifier::Super => "command",
    }
}

pub(crate) fn follow(mut changed: impl FnMut(Hotkeys) + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("keyremap-hotkeys".into())
        .spawn(move || loop {
            let client = PlatformStateClient::from_env();
            if let Some(subscription) = client.subscribe(vec![RuntimeEventKind::HotkeysChanged]) {
                for hotkeys in subscription.events().filter_map(Hotkeys::from_event) {
                    changed(hotkeys);
                }
                changed(Hotkeys::default());
            }
            std::thread::sleep(RESUBSCRIBE);
        });
    if let Err(error) = spawned {
        log::warn!("could not follow qol hotkeys for Karabiner-Elements: {error}");
    }
}

pub(crate) fn from_tray() -> Hotkeys {
    let (tx, rx) = mpsc::channel();
    let _ = std::thread::Builder::new()
        .name("keyremap-hotkeys-snapshot".into())
        .spawn(move || {
            let client = PlatformStateClient::from_env();
            let first = client
                .subscribe(vec![RuntimeEventKind::HotkeysChanged])
                .and_then(|subscription| subscription.events().find_map(Hotkeys::from_event));
            let _ = tx.send(first);
        });
    rx.recv_timeout(SNAPSHOT_WAIT)
        .ok()
        .flatten()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use qol_hotkeys::macos_keycode::{SPACE, TAB};

    use super::*;

    #[test]
    fn each_hotkey_sends_its_press_and_release_to_the_tray_relay() {
        let hotkeys = Hotkeys {
            relay: Some(PathBuf::from("/run/qol/relay.sock")),
            combos: vec![
                HotkeyCombo {
                    mods: vec![HotkeyModifier::Alt],
                    mac_keycode: TAB,
                },
                HotkeyCombo {
                    mods: vec![],
                    mac_keycode: SPACE,
                },
            ],
        };
        let (manipulators, gaps) = hotkeys.manipulators(PhysicalLayout::Ansi);
        assert!(gaps.is_empty());
        assert_eq!(
            manipulators[0],
            json!({
                "type": "basic",
                "from": { "key_code": "tab", "modifiers": { "mandatory": ["option"], "optional": ["caps_lock"] } },
                "to": [{ "send_user_command": {
                    "endpoint": "/run/qol/relay.sock",
                    "payload": { "combo": { "mods": ["alt"], "mac_keycode": TAB }, "pressed": true },
                } }],
                "to_after_key_up": [{ "send_user_command": {
                    "endpoint": "/run/qol/relay.sock",
                    "payload": { "combo": { "mods": ["alt"], "mac_keycode": TAB }, "pressed": false },
                } }],
                "conditions": [{ "type": "variable_if", "name": ACTIVE_VARIABLE, "value": 1 }],
            })
        );
        assert_eq!(
            manipulators[1]["from"],
            json!({ "key_code": "spacebar", "modifiers": { "optional": ["caps_lock"] } })
        );
    }

    #[test]
    fn without_a_relay_the_tray_keeps_its_hotkeys_on_the_tap() {
        let hotkeys = Hotkeys {
            relay: None,
            combos: vec![HotkeyCombo {
                mods: vec![HotkeyModifier::Alt],
                mac_keycode: TAB,
            }],
        };
        assert_eq!(
            hotkeys.manipulators(PhysicalLayout::Ansi),
            (Vec::new(), Vec::new())
        );
    }

    #[test]
    fn a_key_karabiner_cannot_name_is_reported_not_dropped_silently() {
        let hotkeys = Hotkeys {
            relay: Some(PathBuf::from("/run/qol/relay.sock")),
            combos: vec![HotkeyCombo {
                mods: vec![HotkeyModifier::Super],
                mac_keycode: 0x3f,
            }],
        };
        let (manipulators, gaps) = hotkeys.manipulators(PhysicalLayout::Ansi);
        assert!(manipulators.is_empty());
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].rule, "qol hotkey command+0x3f");
    }
}
