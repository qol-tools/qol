use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::plugins::manifest::{ConfigDeclarations, ConfigScope};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSlices {
    #[serde(default, alias = "any")]
    pub core: Value,
    #[serde(default)]
    pub os: Value,
    #[serde(default)]
    pub device: Value,
}

impl ConfigSlices {
    pub fn is_empty(&self) -> bool {
        is_empty_object(&self.core) && is_empty_object(&self.os) && is_empty_object(&self.device)
    }
}

pub fn split_by_scope(config: &Value, scopes: &HashMap<String, ConfigScope>) -> ConfigSlices {
    let Some(map) = config.as_object() else {
        return ConfigSlices::default();
    };

    let mut core = Map::new();
    let mut os = Map::new();
    let mut device = Map::new();

    for (key, value) in map {
        let scope = scopes.get(key).copied().unwrap_or_default();
        let target = match scope {
            ConfigScope::Core => &mut core,
            ConfigScope::Os => &mut os,
            ConfigScope::Device => &mut device,
        };
        target.insert(key.clone(), value.clone());
    }

    ConfigSlices {
        core: Value::Object(core),
        os: Value::Object(os),
        device: Value::Object(device),
    }
}

pub fn split_by_declarations(config: &Value, decl: &ConfigDeclarations) -> ConfigSlices {
    let Some(map) = config.as_object() else {
        return ConfigSlices::default();
    };

    let mut parts = [Map::new(), Map::new(), Map::new()];
    for (key, value) in map {
        let [core, os, device] = split_path(key, value, decl.scope_for(key), decl);
        for (part, piece) in parts.iter_mut().zip([core, os, device]) {
            if let Some(piece) = piece {
                part.insert(key.clone(), piece);
            }
        }
    }

    let [core, os, device] = parts;
    ConfigSlices {
        core: Value::Object(core),
        os: Value::Object(os),
        device: Value::Object(device),
    }
}

fn split_path(
    path: &str,
    value: &Value,
    scope: ConfigScope,
    decl: &ConfigDeclarations,
) -> [Option<Value>; 3] {
    let mut out = [None, None, None];
    let nested = value
        .as_object()
        .filter(|map| !map.is_empty() && has_nested_scope(decl, path));
    let Some(map) = nested else {
        out[slot(scope)] = Some(value.clone());
        return out;
    };
    let mut parts = [Map::new(), Map::new(), Map::new()];
    for (key, child) in map {
        let child_path = format!("{path}.{key}");
        let child_scope = decl.scope.get(&child_path).copied().unwrap_or(scope);
        let pieces = split_path(&child_path, child, child_scope, decl);
        for (part, piece) in parts.iter_mut().zip(pieces) {
            if let Some(piece) = piece {
                part.insert(key.clone(), piece);
            }
        }
    }
    for (index, part) in parts.into_iter().enumerate() {
        if !part.is_empty() {
            out[index] = Some(Value::Object(part));
        }
    }
    out
}

fn has_nested_scope(decl: &ConfigDeclarations, path: &str) -> bool {
    let prefix = format!("{path}.");
    decl.scope.keys().any(|key| key.starts_with(&prefix))
}

fn slot(scope: ConfigScope) -> usize {
    match scope {
        ConfigScope::Core => 0,
        ConfigScope::Os => 1,
        ConfigScope::Device => 2,
    }
}

pub fn merge_slices(slices: &ConfigSlices) -> Value {
    let mut out = Map::new();
    for slice in [&slices.core, &slices.os, &slices.device] {
        if let Some(map) = slice.as_object() {
            merge_into(&mut out, map);
        }
    }
    Value::Object(out)
}

pub fn merge_declared_slices(slices: &ConfigSlices, decl: &ConfigDeclarations) -> Value {
    merge_slices(&ConfigSlices {
        core: synced_part(&slices.core, decl),
        os: synced_part(&slices.os, decl),
        device: slices.device.clone(),
    })
}

fn synced_part(slice: &Value, decl: &ConfigDeclarations) -> Value {
    let split = split_by_declarations(slice, decl);
    merge_slices(&ConfigSlices {
        core: split.core,
        os: split.os,
        device: Value::Object(Map::new()),
    })
}

fn merge_into(target: &mut Map<String, Value>, source: &Map<String, Value>) {
    for (key, value) in source {
        match (target.get_mut(key), value) {
            (Some(Value::Object(existing)), Value::Object(incoming)) => {
                merge_into(existing, incoming)
            }
            _ => {
                target.insert(key.clone(), value.clone());
            }
        }
    }
}

fn is_empty_object(value: &Value) -> bool {
    value.as_object().is_none_or(Map::is_empty)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn scopes(entries: &[(&str, ConfigScope)]) -> HashMap<String, ConfigScope> {
        entries
            .iter()
            .map(|(name, scope)| ((*name).to_string(), *scope))
            .collect()
    }

    #[test]
    fn split_empty_config_yields_empty_slices() {
        let slices = split_by_scope(&json!({}), &HashMap::new());
        assert!(slices.is_empty(), "slices: {slices:?}");
    }

    #[test]
    fn split_non_object_yields_empty_slices() {
        let cases = [
            json!(null),
            json!(true),
            json!(42),
            json!("x"),
            json!([1, 2]),
        ];
        for value in cases {
            let slices = split_by_scope(&value, &HashMap::new());
            assert!(slices.is_empty(), "input {value:?}: {slices:?}");
        }
    }

    #[test]
    fn split_unlisted_keys_default_to_core() {
        let config = json!({ "a": 1, "b": "x" });
        let slices = split_by_scope(&config, &HashMap::new());
        assert_eq!(slices.core, json!({ "a": 1, "b": "x" }));
        assert_eq!(slices.os, json!({}));
        assert_eq!(slices.device, json!({}));
    }

    #[test]
    fn split_routes_each_key_by_declared_scope() {
        let config = json!({
            "presets": ["a"],
            "hotkey": "Super+Space",
            "broker_url": "10.0.0.1",
            "unknown": true,
        });
        let scopes = scopes(&[
            ("presets", ConfigScope::Core),
            ("hotkey", ConfigScope::Os),
            ("broker_url", ConfigScope::Device),
        ]);

        let slices = split_by_scope(&config, &scopes);

        assert_eq!(slices.core, json!({ "presets": ["a"], "unknown": true }));
        assert_eq!(slices.os, json!({ "hotkey": "Super+Space" }));
        assert_eq!(slices.device, json!({ "broker_url": "10.0.0.1" }));
    }

    #[test]
    fn merge_empty_slices_yields_empty_object() {
        let merged = merge_slices(&ConfigSlices::default());
        assert_eq!(merged, json!({}));
    }

    #[test]
    fn merge_unions_keys_across_slices() {
        let slices = ConfigSlices {
            core: json!({ "presets": ["a"] }),
            os: json!({ "hotkey": "Super+Space" }),
            device: json!({ "broker_url": "10.0.0.1" }),
        };
        let merged = merge_slices(&slices);
        assert_eq!(
            merged,
            json!({
                "presets": ["a"],
                "hotkey": "Super+Space",
                "broker_url": "10.0.0.1",
            })
        );
    }

    #[test]
    fn merge_precedence_is_core_then_os_then_device() {
        let cases: &[(ConfigSlices, Value)] = &[
            (
                ConfigSlices {
                    core: json!({ "k": "core" }),
                    os: json!({ "k": "os" }),
                    device: json!({}),
                },
                json!({ "k": "os" }),
            ),
            (
                ConfigSlices {
                    core: json!({ "k": "core" }),
                    os: json!({}),
                    device: json!({ "k": "device" }),
                },
                json!({ "k": "device" }),
            ),
            (
                ConfigSlices {
                    core: json!({}),
                    os: json!({ "k": "os" }),
                    device: json!({ "k": "device" }),
                },
                json!({ "k": "device" }),
            ),
        ];
        for (slices, expected) in cases {
            assert_eq!(merge_slices(slices), *expected, "slices {slices:?}");
        }
    }

    #[test]
    fn split_then_merge_roundtrips_object() {
        let config = json!({
            "presets": ["a", "b"],
            "hotkey": "Super+Space",
            "broker_url": "10.0.0.1",
            "extra": { "nested": true },
        });
        let scopes = scopes(&[
            ("presets", ConfigScope::Core),
            ("hotkey", ConfigScope::Os),
            ("broker_url", ConfigScope::Device),
        ]);

        let slices = split_by_scope(&config, &scopes);
        let merged = merge_slices(&slices);

        assert_eq!(merged, config);
    }

    fn declarations(
        default_scope: Option<ConfigScope>,
        per_field: &[(&str, ConfigScope)],
    ) -> ConfigDeclarations {
        ConfigDeclarations {
            default_scope,
            scope: per_field
                .iter()
                .map(|(k, s)| ((*k).to_string(), *s))
                .collect(),
        }
    }

    #[test]
    fn split_by_declarations_honors_per_field_scope_then_default_scope() {
        let config = json!({
            "presets": ["a"],
            "hotkey": "Super+Space",
            "broker_url": "10.0.0.1",
            "unscoped_field": "x",
        });
        let decl = declarations(
            Some(ConfigScope::Os),
            &[
                ("presets", ConfigScope::Core),
                ("broker_url", ConfigScope::Device),
            ],
        );

        let slices = split_by_declarations(&config, &decl);

        assert_eq!(slices.core, json!({ "presets": ["a"] }));
        assert_eq!(
            slices.os,
            json!({ "hotkey": "Super+Space", "unscoped_field": "x" }),
            "unspecified fields follow default_scope (os) here"
        );
        assert_eq!(slices.device, json!({ "broker_url": "10.0.0.1" }));
    }

    #[test]
    fn split_by_declarations_without_default_scope_routes_unspecified_fields_to_core() {
        let config = json!({ "presets": ["a"], "broker_url": "10.0.0.1" });
        let decl = declarations(None, &[("broker_url", ConfigScope::Device)]);
        let slices = split_by_declarations(&config, &decl);
        assert_eq!(slices.core, json!({ "presets": ["a"] }));
        assert_eq!(slices.device, json!({ "broker_url": "10.0.0.1" }));
        assert_eq!(slices.os, json!({}));
    }

    #[test]
    fn config_slices_deserializes_legacy_any_field_into_core_slot() {
        let raw = json!({ "any": { "presets": ["a"] }, "os": { "k": "v" } });
        let slices: ConfigSlices = serde_json::from_value(raw).unwrap();
        assert_eq!(
            slices.core,
            json!({ "presets": ["a"] }),
            "legacy `any` field on persisted slices must round-trip into the renamed `core` slot"
        );
        assert_eq!(slices.os, json!({ "k": "v" }));
    }

    #[test]
    fn split_by_declarations_routes_dotted_paths_inside_shared_objects() {
        let config = json!({
            "audio": { "enabled": true, "inputs": ["mic"], "mic_device": "usb-mic" },
            "video": { "crf": 18 },
        });
        let decl = declarations(
            Some(ConfigScope::Os),
            &[("audio.mic_device", ConfigScope::Device)],
        );

        let slices = split_by_declarations(&config, &decl);

        assert_eq!(slices.core, json!({}));
        assert_eq!(
            slices.os,
            json!({ "audio": { "enabled": true, "inputs": ["mic"] }, "video": { "crf": 18 } })
        );
        assert_eq!(
            slices.device,
            json!({ "audio": { "mic_device": "usb-mic" } })
        );
        assert_eq!(merge_slices(&slices), config);
    }

    #[test]
    fn split_by_declarations_nested_paths_inherit_the_parent_scope() {
        let config = json!({
            "backend": { "serial_port": "/dev/ttyUSB0", "channel": 11, "deep": { "a": 1 } },
        });
        let decl = declarations(
            None,
            &[
                ("backend", ConfigScope::Os),
                ("backend.serial_port", ConfigScope::Device),
            ],
        );

        let slices = split_by_declarations(&config, &decl);

        assert_eq!(
            slices.os,
            json!({ "backend": { "channel": 11, "deep": { "a": 1 } } })
        );
        assert_eq!(
            slices.device,
            json!({ "backend": { "serial_port": "/dev/ttyUSB0" } })
        );
        assert_eq!(slices.core, json!({}));
    }

    #[test]
    fn merge_combines_nested_objects_leaf_by_leaf() {
        let slices = ConfigSlices {
            core: json!({ "audio": { "enabled": true, "mic_device": "stale" } }),
            os: json!({}),
            device: json!({ "audio": { "mic_device": "usb-mic" }, "volume": 40 }),
        };
        assert_eq!(
            merge_slices(&slices),
            json!({ "audio": { "enabled": true, "mic_device": "usb-mic" }, "volume": 40 })
        );
    }

    #[test]
    fn merge_declared_slices_never_reads_device_paths_from_synced_slices() {
        let decl = declarations(
            None,
            &[
                ("adapter", ConfigScope::Device),
                ("audio.mic_device", ConfigScope::Device),
            ],
        );
        let cases = [
            (
                json!({ "adapter": "84:2F:57:43:D1:14", "auto_reconnect": true }),
                json!({}),
                json!({}),
                json!({ "auto_reconnect": true }),
            ),
            (
                json!({ "adapter": "84:2F:57:43:D1:14" }),
                json!({ "adapter": "AA:AA:AA:AA:AA:AA" }),
                json!({ "adapter": "10:91:D1:28:D5:A6" }),
                json!({ "adapter": "10:91:D1:28:D5:A6" }),
            ),
            (
                json!({ "audio": { "enabled": true, "mic_device": "other-machine" } }),
                json!({ "audio": { "mic_device": "other-os-machine" } }),
                json!({}),
                json!({ "audio": { "enabled": true } }),
            ),
        ];
        for (core, os, device, expected) in cases {
            let slices = ConfigSlices { core, os, device };
            assert_eq!(
                merge_declared_slices(&slices, &decl),
                expected,
                "slices {slices:?}"
            );
        }
    }
}
