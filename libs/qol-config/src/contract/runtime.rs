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
    #[serde(default, rename = "stream")]
    pub streams: IndexMap<String, StreamSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ActionSpec {
    pub description: String,
    #[serde(default)]
    pub confirm: Option<String>,
    #[serde(default)]
    pub input: Option<IndexMap<String, String>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct QuerySpec {
    pub description: String,
    pub poll_interval_ms: u64,
}

const STREAM_THROTTLE_MIN: u64 = 16;
const STREAM_THROTTLE_MAX: u64 = 1000;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct StreamSpec {
    pub description: String,
    #[serde(deserialize_with = "deserialize_throttle_ms")]
    pub throttle_ms: u64,
    #[serde(default)]
    pub initial_query: Option<String>,
}

fn deserialize_throttle_ms<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    let raw = u64::deserialize(d)?;
    Ok(raw.clamp(STREAM_THROTTLE_MIN, STREAM_THROTTLE_MAX))
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
    let mut all_names = IndexMap::<&str, &str>::new();
    validate_names(&spec.actions, "action", &mut all_names)?;
    validate_names(&spec.queries, "query", &mut all_names)?;
    validate_names(&spec.streams, "stream", &mut all_names)?;
    validate_initial_queries(spec)
}

fn validate_names<'a, V>(
    map: &'a IndexMap<String, V>,
    kind: &'static str,
    all_names: &mut IndexMap<&'a str, &'static str>,
) -> Result<(), ParseRuntimeSpecError> {
    for name in map.keys() {
        if !is_valid_runable_name(name) {
            return Err(ParseRuntimeSpecError::Validation(format!(
                "invalid {kind} name: {name}"
            )));
        }
        if let Some(existing) = all_names.get(name.as_str()) {
            return Err(ParseRuntimeSpecError::Validation(format!(
                "name collision between {existing} and {kind}: {name}"
            )));
        }
        all_names.insert(name.as_str(), kind);
    }
    Ok(())
}

fn validate_initial_queries(spec: &RuntimeSpec) -> Result<(), ParseRuntimeSpecError> {
    for (name, stream) in &spec.streams {
        let Some(ref qname) = stream.initial_query else {
            continue;
        };
        if !spec.queries.contains_key(qname) {
            return Err(ParseRuntimeSpecError::Validation(format!(
                "stream {name} initial_query references undeclared query: {qname}"
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
    fn parses_streams() {
        let input = r#"
schema_version = 1

[stream.live_color]
description = "Real-time color control"
throttle_ms = 100

[query.current_color]
description = "Current color"
poll_interval_ms = 1000

[stream.motor]
description = "Motor position"
throttle_ms = 50
initial_query = "current_color"
"#;
        let spec = parse_runtime_spec_str(input).expect("parse");
        assert_eq!(spec.streams.len(), 2, "two streams");
        let color = &spec.streams["live_color"];
        assert_eq!(color.throttle_ms, 100);
        assert!(color.initial_query.is_none());
        let motor = &spec.streams["motor"];
        assert_eq!(motor.throttle_ms, 50);
        assert_eq!(motor.initial_query.as_deref(), Some("current_color"));
    }

    #[test]
    fn clamps_throttle_ms() {
        let cases = [
            (0, 16, "below min"),
            (5, 16, "below min"),
            (2000, 1000, "above max"),
            (500, 500, "in range"),
        ];
        for (input_ms, expected, label) in cases {
            let input = format!(
                "schema_version = 1\n\n[stream.s]\ndescription = \"test\"\nthrottle_ms = {input_ms}\n"
            );
            let spec = parse_runtime_spec_str(&input).expect("parse");
            assert_eq!(spec.streams["s"].throttle_ms, expected, "{label}");
        }
    }

    #[test]
    fn rejects_stream_name_collision_with_action() {
        let input = r#"
schema_version = 1

[action.foo]
description = "action"

[stream.foo]
description = "stream"
throttle_ms = 100
"#;
        let result = parse_runtime_spec_str(input);
        assert!(
            result.is_err(),
            "stream-action collision should be rejected"
        );
    }

    #[test]
    fn rejects_stream_with_dangling_initial_query() {
        let input = r#"
schema_version = 1

[stream.s]
description = "test"
throttle_ms = 100
initial_query = "nonexistent"
"#;
        let result = parse_runtime_spec_str(input);
        assert!(result.is_err(), "dangling initial_query should be rejected");
    }

    #[test]
    fn parses_action_with_input() {
        let input = r#"
schema_version = 1

[action.remove_device]
description = "Remove a device"
input = { ieee = "string" }
"#;
        let spec = parse_runtime_spec_str(input).expect("parse");
        let action = &spec.actions["remove_device"];
        let input_schema = action.input.as_ref().expect("input present");
        assert_eq!(input_schema["ieee"], "string");
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

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;

    fn valid_name_strategy() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{0,31}".prop_map(String::from)
    }

    proptest! {
        #[test]
        fn valid_names_always_parse_as_actions(name in valid_name_strategy()) {
            let input = format!(
                "schema_version = 1\n\n[action.{name}]\ndescription = \"test\"\n"
            );
            let result = parse_runtime_spec_str(&input);
            prop_assert!(result.is_ok(), "valid name {name} should parse: {:?}", result);
            let spec = result.unwrap();
            prop_assert!(spec.actions.contains_key(&name));
        }

        #[test]
        fn is_valid_runable_name_matches_regex(name in "[a-zA-Z0-9_ ]{0,32}") {
            let valid = is_valid_runable_name(&name);
            let expected = !name.is_empty()
                && name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            prop_assert_eq!(valid, expected, "mismatch on {:?}", name);
        }
    }
}
