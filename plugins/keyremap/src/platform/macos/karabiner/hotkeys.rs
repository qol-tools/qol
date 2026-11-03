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
        if self.combos.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let mut manipulators: Vec<Value> = MODIFIERS
            .iter()
            .flat_map(|&modifier| sides(modifier))
            .map(held_tracker)
            .collect();
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
            let send = |pressed: bool| {
                json!([{ "send_user_command": {
                    "endpoint": relay,
                    "payload": HotkeyPress { combo: combo.clone(), pressed },
                } }])
            };
            for conditions in held_conditions(&combo.mods) {
                manipulators.push(json!({
                    "type": "basic",
                    "from": { "key_code": key_code, "modifiers": { "optional": ["any"] } },
                    "to": send(true),
                    "to_after_key_up": send(false),
                    "conditions": conditions,
                }));
            }
        }
        (manipulators, gaps)
    }
}

const MODIFIERS: [HotkeyModifier; 4] = [
    HotkeyModifier::Shift,
    HotkeyModifier::Ctrl,
    HotkeyModifier::Alt,
    HotkeyModifier::Super,
];

fn sides(modifier: HotkeyModifier) -> [&'static str; 2] {
    match modifier {
        HotkeyModifier::Shift => ["left_shift", "right_shift"],
        HotkeyModifier::Ctrl => ["left_control", "right_control"],
        HotkeyModifier::Alt => ["left_option", "right_option"],
        HotkeyModifier::Super => ["left_command", "right_command"],
    }
}

fn held_variable(key_code: &str) -> String {
    format!("qol_hotkey_held_{key_code}")
}

fn held_tracker(key_code: &str) -> Value {
    json!({
        "type": "basic",
        "from": { "key_code": key_code, "modifiers": { "optional": ["any"] } },
        "to": [
            { "set_variable": { "name": held_variable(key_code), "value": 1, "key_up_value": 0 } },
            { "key_code": key_code },
        ],
        "conditions": [active()],
    })
}

fn active() -> Value {
    json!({ "type": "variable_if", "name": ACTIVE_VARIABLE, "value": 1 })
}

fn held_if(key_code: &str, value: u8) -> Value {
    json!({ "type": "variable_if", "name": held_variable(key_code), "value": value })
}

fn held_conditions(mods: &[HotkeyModifier]) -> Vec<Vec<Value>> {
    let mut variants = vec![vec![active()]];
    for modifier in MODIFIERS {
        let [left, right] = sides(modifier);
        if mods.contains(&modifier) {
            variants = variants
                .into_iter()
                .flat_map(|conditions| {
                    [left, right].map(|side| {
                        let mut conditions = conditions.clone();
                        conditions.push(held_if(side, 1));
                        conditions
                    })
                })
                .collect();
        } else {
            for conditions in &mut variants {
                conditions.push(held_if(left, 0));
                conditions.push(held_if(right, 0));
            }
        }
    }
    variants
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

pub(crate) fn from_tray() -> Option<Hotkeys> {
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
    rx.recv_timeout(SNAPSHOT_WAIT).ok().flatten()
}

#[cfg(test)]
mod tests {
    use qol_hotkeys::macos_keycode::{SPACE, TAB};

    use super::*;

    fn relayed(combos: Vec<HotkeyCombo>) -> Vec<Value> {
        let hotkeys = Hotkeys {
            relay: Some(PathBuf::from("/run/qol/relay.sock")),
            combos,
        };
        let (manipulators, gaps) = hotkeys.manipulators(PhysicalLayout::Ansi);
        assert!(gaps.is_empty());
        manipulators
    }

    fn alt_tab() -> HotkeyCombo {
        HotkeyCombo {
            mods: vec![HotkeyModifier::Alt],
            mac_keycode: TAB,
        }
    }

    fn held(side: &str, value: u8) -> Value {
        json!({ "type": "variable_if", "name": format!("qol_hotkey_held_{side}"), "value": value })
    }

    #[test]
    fn every_modifier_key_passes_through_and_records_that_it_is_held() {
        let manipulators = relayed(vec![alt_tab()]);
        assert_eq!(
            manipulators[4],
            json!({
                "type": "basic",
                "from": { "key_code": "left_option", "modifiers": { "optional": ["any"] } },
                "to": [
                    { "set_variable": { "name": "qol_hotkey_held_left_option", "value": 1, "key_up_value": 0 } },
                    { "key_code": "left_option" },
                ],
                "conditions": [{ "type": "variable_if", "name": ACTIVE_VARIABLE, "value": 1 }],
            })
        );
        let tracked: Vec<&str> = manipulators[..8]
            .iter()
            .map(|manipulator| manipulator["from"]["key_code"].as_str().unwrap())
            .collect();
        assert_eq!(
            tracked,
            [
                "left_shift",
                "right_shift",
                "left_control",
                "right_control",
                "left_option",
                "right_option",
                "left_command",
                "right_command",
            ]
        );
    }

    #[test]
    fn a_hotkey_matches_on_held_variables_so_karabiner_never_lifts_the_held_modifier() {
        let manipulators = relayed(vec![alt_tab()]);
        assert_eq!(manipulators.len(), 10, "one manipulator per side of option");
        let send = |pressed: bool| {
            json!([{ "send_user_command": {
                "endpoint": "/run/qol/relay.sock",
                "payload": { "combo": { "mods": ["alt"], "mac_keycode": TAB }, "pressed": pressed },
            } }])
        };
        let not_held = |conditions: &mut Vec<Value>, sides: [&str; 2]| {
            conditions.extend(sides.map(|side| held(side, 0)));
        };
        for (manipulator, side) in manipulators[8..]
            .iter()
            .zip(["left_option", "right_option"])
        {
            let mut conditions =
                vec![json!({ "type": "variable_if", "name": ACTIVE_VARIABLE, "value": 1 })];
            not_held(&mut conditions, ["left_shift", "right_shift"]);
            not_held(&mut conditions, ["left_control", "right_control"]);
            conditions.push(held(side, 1));
            not_held(&mut conditions, ["left_command", "right_command"]);
            assert_eq!(
                *manipulator,
                json!({
                    "type": "basic",
                    "from": { "key_code": "tab", "modifiers": { "optional": ["any"] } },
                    "to": send(true),
                    "to_after_key_up": send(false),
                    "conditions": conditions,
                })
            );
        }
    }

    #[test]
    fn each_extra_modifier_doubles_the_side_variants_and_a_bare_key_needs_one() {
        let shift_super_r = HotkeyCombo {
            mods: vec![HotkeyModifier::Shift, HotkeyModifier::Super],
            mac_keycode: qol_hotkeys::macos_keycode::ANSI_R,
        };
        let space = HotkeyCombo {
            mods: vec![],
            mac_keycode: SPACE,
        };
        let manipulators = relayed(vec![shift_super_r, space]);
        assert_eq!(manipulators.len(), 8 + 4 + 1);
        assert_eq!(manipulators[12]["from"]["key_code"], "spacebar");
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
        assert_eq!(manipulators.len(), 8, "only the modifier trackers");
        assert_eq!(gaps.len(), 1);
        assert_eq!(gaps[0].rule, "qol hotkey command+0x3f");
    }
}
