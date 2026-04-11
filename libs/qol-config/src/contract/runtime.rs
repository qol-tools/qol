use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug)]
pub enum ParseRuntimeSpecError {
    Io(std::io::Error),
    Toml(toml::de::Error),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RuntimeSpec {
    pub schema_version: u32,
    #[serde(default, rename = "action")]
    pub actions: IndexMap<String, ActionSpec>,
    #[serde(default, rename = "query")]
    pub queries: IndexMap<String, QuerySpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ActionSpec {
    pub description: String,
    #[serde(default)]
    pub confirm: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct QuerySpec {
    pub description: String,
    pub poll_interval_ms: u64,
}

pub fn parse_runtime_spec(path: impl AsRef<Path>) -> Result<RuntimeSpec, ParseRuntimeSpecError> {
    let raw = std::fs::read_to_string(path).map_err(ParseRuntimeSpecError::Io)?;
    parse_runtime_spec_str(&raw).map_err(ParseRuntimeSpecError::Toml)
}

pub fn parse_runtime_spec_str(input: &str) -> Result<RuntimeSpec, toml::de::Error> {
    toml::from_str(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_runtime_with_actions_and_queries() {
        let input = r#"
schema_version = 1

[action.pair_device]
description = "Pair a Zigbee device"
confirm = "Start pairing mode?"

[action.remove_device]
description = "Remove a paired device"

[query.list_devices]
description = "List all paired devices"
poll_interval_ms = 2000

[query.connection_status]
description = "Current coordinator state"
poll_interval_ms = 1000
"#;
        let spec = parse_runtime_spec_str(input).expect("parse");
        assert_eq!(spec.schema_version, 1);
        assert_eq!(spec.actions.len(), 2, "two actions");
        assert_eq!(spec.queries.len(), 2, "two queries");
        let pair = spec
            .actions
            .get("pair_device")
            .expect("pair_device present");
        assert_eq!(pair.description, "Pair a Zigbee device");
        assert_eq!(pair.confirm.as_deref(), Some("Start pairing mode?"));
        let list = spec
            .queries
            .get("list_devices")
            .expect("list_devices present");
        assert_eq!(list.poll_interval_ms, 2000);
    }

    #[test]
    fn parses_empty_runtime() {
        let input = "schema_version = 1\n";
        let spec = parse_runtime_spec_str(input).expect("parse");
        assert_eq!(spec.schema_version, 1);
        assert!(spec.actions.is_empty(), "no actions");
        assert!(spec.queries.is_empty(), "no queries");
    }
}
