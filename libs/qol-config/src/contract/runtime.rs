use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug)]
pub enum ParseRuntimeSpecError {
    Io(std::io::Error),
    Toml(toml::de::Error),
    Validation(String),
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

pub fn is_valid_runable_name(name: &str) -> bool {
    let mut chars = name.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn validate_runtime_spec(spec: &RuntimeSpec) -> Result<(), ParseRuntimeSpecError> {
    for name in spec.actions.keys() {
        if !is_valid_runable_name(name) {
            return Err(ParseRuntimeSpecError::Validation(format!(
                "invalid action name: {name}"
            )));
        }
    }
    for name in spec.queries.keys() {
        if !is_valid_runable_name(name) {
            return Err(ParseRuntimeSpecError::Validation(format!(
                "invalid query name: {name}"
            )));
        }
    }
    for name in spec.queries.keys() {
        if spec.actions.contains_key(name) {
            return Err(ParseRuntimeSpecError::Validation(format!(
                "name collision between action and query: {name}"
            )));
        }
    }
    Ok(())
}

pub fn parse_runtime_spec(path: impl AsRef<Path>) -> Result<RuntimeSpec, ParseRuntimeSpecError> {
    let raw = std::fs::read_to_string(path).map_err(ParseRuntimeSpecError::Io)?;
    parse_runtime_spec_str(&raw)
}

pub fn parse_runtime_spec_str(input: &str) -> Result<RuntimeSpec, ParseRuntimeSpecError> {
    let spec: RuntimeSpec = toml::from_str(input).map_err(ParseRuntimeSpecError::Toml)?;
    validate_runtime_spec(&spec)?;
    Ok(spec)
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

    #[test]
    fn rejects_invalid_action_names() {
        let cases = [
            ("has-dash", "dash not allowed"),
            ("InvalidCase", "uppercase not allowed"),
            ("1starts_with_digit", "cannot start with digit"),
        ];
        for (name, why) in cases {
            let input = format!("schema_version = 1\n\n[action.{name}]\ndescription = \"test\"\n");
            let result = parse_runtime_spec_str(&input);
            assert!(result.is_err(), "should reject {name}: {why}");
        }
    }

    #[test]
    fn accepts_valid_action_names() {
        let cases = ["pair_device", "a", "snake_case_name", "abc123"];
        for name in cases {
            let input = format!("schema_version = 1\n\n[action.{name}]\ndescription = \"test\"\n");
            let spec = parse_runtime_spec_str(&input).expect("parse");
            assert!(spec.actions.contains_key(name), "{name} should parse");
        }
    }

    #[test]
    fn rejects_name_collision_between_action_and_query() {
        let input = r#"
schema_version = 1

[action.foo]
description = "action foo"

[query.foo]
description = "query foo"
poll_interval_ms = 1000
"#;
        let result = parse_runtime_spec_str(input);
        assert!(
            result.is_err(),
            "same name in action and query should be rejected"
        );
    }
}
