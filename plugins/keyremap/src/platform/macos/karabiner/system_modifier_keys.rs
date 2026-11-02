use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const PREFIX: &str = "com.apple.keyboard.modifiermapping.";
const DST: &str = "HIDKeyboardModifierMappingDst";
const SRC: &str = "HIDKeyboardModifierMappingSrc";
const KEYBOARD_PAGE: u64 = 0x7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DeviceModifiers {
    pub(crate) identifiers: Value,
    pub(crate) simple_modifications: Vec<Value>,
}

pub(crate) fn read() -> Vec<DeviceModifiers> {
    let output = Command::new("defaults")
        .args(["-currentHost", "read", "-g"])
        .output();
    match output {
        Ok(output) if output.status.success() => parse(&String::from_utf8_lossy(&output.stdout)),
        Ok(output) => {
            log::debug!(
                "no System Settings modifier keys to carry into Karabiner-Elements: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            Vec::new()
        }
        Err(error) => {
            log::debug!("could not read System Settings modifier keys: {error}");
            Vec::new()
        }
    }
}

pub(crate) fn parse(text: &str) -> Vec<DeviceModifiers> {
    let mut devices = Vec::new();
    let mut current: Option<DeviceModifiers> = None;
    let mut dst = None;
    let mut src = None;
    for line in text.lines().map(str::trim) {
        if let Some(identifiers) = device_line(line) {
            current = Some(DeviceModifiers {
                identifiers,
                simple_modifications: Vec::new(),
            });
            continue;
        }
        let Some(device) = current.as_mut() else {
            continue;
        };
        if let Some(value) = number_after(line, DST) {
            dst = Some(value);
        } else if let Some(value) = number_after(line, SRC) {
            src = Some(value);
        } else if line.starts_with('}') {
            let entry = match (src.take(), dst.take()) {
                (Some(from), Some(to)) => modification(from, to),
                _ => None,
            };
            let mapped = |entry: &Value| {
                device
                    .simple_modifications
                    .iter()
                    .any(|existing| existing.get("from") == entry.get("from"))
            };
            if let Some(entry) = entry.filter(|entry| !mapped(entry)) {
                device.simple_modifications.push(entry);
            }
        } else if line.starts_with(");") {
            devices.extend(
                current
                    .take()
                    .filter(|device| !device.simple_modifications.is_empty()),
            );
        }
    }
    devices
}

fn device_line(line: &str) -> Option<Value> {
    let rest = line.strip_prefix('"')?.strip_prefix(PREFIX)?;
    let id = rest.split('"').next()?;
    let mut parts = id.split('-').map(str::parse::<u64>);
    let (Some(Ok(vendor)), Some(Ok(product))) = (parts.next(), parts.next()) else {
        return None;
    };
    Some(if vendor == 0 && product == 0 {
        json!({ "is_keyboard": true })
    } else {
        json!({ "is_keyboard": true, "product_id": product, "vendor_id": vendor })
    })
}

fn number_after(line: &str, key: &str) -> Option<u64> {
    line.strip_prefix(key)?
        .trim_start()
        .strip_prefix('=')?
        .trim()
        .trim_end_matches(';')
        .parse()
        .ok()
}

fn modification(from: u64, to: u64) -> Option<Value> {
    let from_key = key(from)?;
    if from == to {
        return None;
    }
    let to_key = if to == 0 {
        json!({ "key_code": "vk_none" })
    } else {
        key(to)?
    };
    Some(json!({ "from": from_key, "to": [to_key] }))
}

fn key(usage: u64) -> Option<Value> {
    if usage == 0xFF_0000_0003 || usage == 0xFF01_0000_0003 {
        return Some(json!({ "apple_vendor_top_case_key_code": "keyboard_fn" }));
    }
    if usage >> 32 != KEYBOARD_PAGE {
        return None;
    }
    let name = match usage & 0xFFFF_FFFF {
        0x29 => "escape",
        0x39 => "caps_lock",
        0xE0 => "left_control",
        0xE1 => "left_shift",
        0xE2 => "left_option",
        0xE3 => "left_command",
        0xE4 => "right_control",
        0xE5 => "right_shift",
        0xE6 => "right_option",
        0xE7 => "right_command",
        _ => return None,
    };
    Some(json!({ "key_code": name }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULTS: &str = r#"{
    AppleLanguages =     (
        "en-NO"
    );
    "com.apple.keyboard.modifiermapping.0-0-0" =     (
                {
            HIDKeyboardModifierMappingDst = 30064771298;
            HIDKeyboardModifierMappingSrc = 30064771299;
        },
                {
            HIDKeyboardModifierMappingDst = 30064771296;
            HIDKeyboardModifierMappingSrc = 280379760050179;
        },
                {
            HIDKeyboardModifierMappingDst = 1095216660483;
            HIDKeyboardModifierMappingSrc = 30064771129;
        },
                {
            HIDKeyboardModifierMappingDst = 0;
            HIDKeyboardModifierMappingSrc = 30064771300;
        },
                {
            HIDKeyboardModifierMappingDst = 30064771300;
            HIDKeyboardModifierMappingSrc = 1095216660483;
        }
    );
    "com.apple.keyboard.modifiermapping.1133-45952-0" =     (
                {
            HIDKeyboardModifierMappingDst = 30064771302;
            HIDKeyboardModifierMappingSrc = 30064771303;
        },
                {
            HIDKeyboardModifierMappingDst = 30064771303;
            HIDKeyboardModifierMappingSrc = 30064771303;
        }
    );
    "com.apple.keyboard.modifiermapping.5-6-0" =     (
    );
    KeyRepeat = 2;
}"#;

    fn simple(from: Value, to: Value) -> Value {
        json!({ "from": from, "to": [to] })
    }

    #[test]
    fn system_settings_modifier_keys_become_per_keyboard_simple_modifications_first_one_wins() {
        let devices = parse(DEFAULTS);
        let fn_key = json!({ "apple_vendor_top_case_key_code": "keyboard_fn" });
        assert_eq!(
            devices,
            vec![
                DeviceModifiers {
                    identifiers: json!({ "is_keyboard": true }),
                    simple_modifications: vec![
                        simple(
                            json!({ "key_code": "left_command" }),
                            json!({ "key_code": "left_option" })
                        ),
                        simple(fn_key.clone(), json!({ "key_code": "left_control" })),
                        simple(json!({ "key_code": "caps_lock" }), fn_key),
                        simple(
                            json!({ "key_code": "right_control" }),
                            json!({ "key_code": "vk_none" })
                        ),
                    ],
                },
                DeviceModifiers {
                    identifiers: json!({ "is_keyboard": true, "product_id": 45952, "vendor_id": 1133 }),
                    simple_modifications: vec![simple(
                        json!({ "key_code": "right_command" }),
                        json!({ "key_code": "right_option" })
                    )],
                },
            ]
        );
    }

    #[test]
    fn no_mappings_reads_as_nothing() {
        assert!(parse("{\n    KeyRepeat = 2;\n}").is_empty());
        assert!(parse("").is_empty());
    }
}
