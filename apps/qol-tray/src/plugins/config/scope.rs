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

    let mut core = Map::new();
    let mut os = Map::new();
    let mut device = Map::new();

    for (key, value) in map {
        let target = match decl.scope_for(key) {
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

pub fn merge_slices(slices: &ConfigSlices) -> Value {
    let mut out = Map::new();
    for slice in [&slices.core, &slices.os, &slices.device] {
        let Some(map) = slice.as_object() else {
            continue;
        };
        for (key, value) in map {
            out.insert(key.clone(), value.clone());
        }
    }
    Value::Object(out)
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
}
