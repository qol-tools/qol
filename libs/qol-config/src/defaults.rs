use crate::contract::{parse_spec_str, ConfigSpec, FieldDefault, FieldKind};
use crate::validation::{validate_spec_collect, ValidationError};
use indexmap::IndexMap;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};

pub fn defaults_json_from_contract(contract: &str) -> Result<Value, Vec<ValidationError>> {
    let spec = parse_contract(contract)?;
    defaults_json_from_spec(&spec)
}

pub fn defaults_json_from_spec(spec: &ConfigSpec) -> Result<Value, Vec<ValidationError>> {
    let mut errors = validate_spec_collect(spec);
    if !errors.is_empty() {
        return Err(errors);
    }

    let mut root = Map::new();
    for (id, field) in &spec.fields {
        if !field_has_stored_value(field.kind) {
            continue;
        }
        let Some(default) = &field.default else {
            continue;
        };
        let key = field.config_key.as_deref().unwrap_or(id);
        if let Err(error) = insert_path(&mut root, key, field_default_to_json(default)) {
            errors.push(ValidationError::new(
                format!("field.{id}.config_key"),
                error,
            ));
        }
    }

    if errors.is_empty() {
        Ok(Value::Object(root))
    } else {
        Err(errors)
    }
}

pub fn typed_defaults_from_contract<T: DeserializeOwned>(
    contract: &str,
) -> Result<T, Vec<ValidationError>> {
    let spec = parse_contract(contract)?;
    typed_defaults_from_spec(&spec)
}

pub fn typed_defaults_from_spec<T: DeserializeOwned>(
    spec: &ConfigSpec,
) -> Result<T, Vec<ValidationError>> {
    let defaults = defaults_json_from_spec(spec)?;
    serde_json::from_value(defaults).map_err(|error| {
        vec![ValidationError::new(
            "defaults",
            format!("failed to deserialize defaults: {error}"),
        )]
    })
}

pub fn validate_contract_defaults_match_type<T>(contract: &str) -> Result<(), Vec<ValidationError>>
where
    T: DeserializeOwned + Serialize,
{
    let spec = parse_contract(contract)?;
    validate_defaults_match_type::<T>(&spec)
}

pub fn validate_defaults_match_type<T>(spec: &ConfigSpec) -> Result<(), Vec<ValidationError>>
where
    T: DeserializeOwned + Serialize,
{
    let defaults = defaults_json_from_spec(spec)?;
    let typed = serde_json::from_value::<T>(defaults.clone()).map_err(|error| {
        vec![ValidationError::new(
            "defaults",
            format!("failed to deserialize defaults: {error}"),
        )]
    })?;
    let typed_json = serde_json::to_value(typed).map_err(|error| {
        vec![ValidationError::new(
            "defaults",
            format!("failed to serialize typed defaults: {error}"),
        )]
    })?;
    let mut errors = Vec::new();
    validate_json_contains(&typed_json, &defaults, "defaults", &mut errors);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn deserialize_with_contract_defaults<T: DeserializeOwned>(
    contract: &str,
    overrides: Value,
) -> Result<T, Vec<ValidationError>> {
    let merged = merge_json_defaults(defaults_json_from_contract(contract)?, overrides);
    serde_json::from_value(merged).map_err(|error| {
        vec![ValidationError::new(
            "config",
            format!("failed to deserialize config: {error}"),
        )]
    })
}

fn parse_contract(contract: &str) -> Result<ConfigSpec, Vec<ValidationError>> {
    parse_spec_str(contract).map_err(|error| {
        vec![ValidationError::new(
            "contract",
            format!("failed to parse config contract: {error}"),
        )]
    })
}

pub(crate) fn merge_json_defaults(defaults: Value, overrides: Value) -> Value {
    match (defaults, overrides) {
        (Value::Object(mut defaults), Value::Object(overrides)) => {
            for (key, override_value) in overrides {
                match defaults.remove(&key) {
                    Some(default_value) => {
                        defaults.insert(key, merge_json_defaults(default_value, override_value));
                    }
                    None => {
                        defaults.insert(key, override_value);
                    }
                }
            }
            Value::Object(defaults)
        }
        (_, override_value) => override_value,
    }
}

fn field_has_stored_value(kind: FieldKind) -> bool {
    !matches!(
        kind,
        FieldKind::Action | FieldKind::List | FieldKind::Status | FieldKind::QrCode
    )
}

fn insert_path(root: &mut Map<String, Value>, path: &str, value: Value) -> Result<(), String> {
    let parts: Vec<&str> = path.split('.').collect();
    insert_path_parts(root, &parts, value)
}

fn insert_path_parts(
    current: &mut Map<String, Value>,
    parts: &[&str],
    value: Value,
) -> Result<(), String> {
    let Some((head, tail)) = parts.split_first() else {
        return Err("empty config key".to_string());
    };
    if tail.is_empty() {
        if current.insert((*head).to_string(), value).is_some() {
            return Err(format!("duplicate config key {}", parts.join(".")));
        }
        return Ok(());
    }

    let entry = current
        .entry((*head).to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let Some(object) = entry.as_object_mut() else {
        return Err(format!(
            "config key {} collides with a scalar",
            parts.join(".")
        ));
    };
    insert_path_parts(object, tail, value)
}

fn field_default_to_json(default: &FieldDefault) -> Value {
    match default {
        FieldDefault::Boolean(value) => Value::Bool(*value),
        FieldDefault::String(value) => Value::String(value.clone()),
        FieldDefault::Number(value) => number_to_json(*value),
        FieldDefault::StringArray(values) => values
            .iter()
            .cloned()
            .map(Value::String)
            .collect::<Vec<_>>()
            .into(),
        FieldDefault::ObjectArray(values) => values
            .iter()
            .map(object_item_to_json)
            .collect::<Vec<_>>()
            .into(),
        FieldDefault::ObjectMap(values) => object_map_to_json(values),
    }
}

fn number_to_json(value: f64) -> Value {
    if value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64 {
        return Value::Number(serde_json::Number::from(value as i64));
    }
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

fn object_item_to_json(item: &IndexMap<String, FieldDefault>) -> Value {
    Value::Object(
        item.iter()
            .map(|(key, value)| (key.clone(), field_default_to_json(value)))
            .collect(),
    )
}

fn object_map_to_json(values: &IndexMap<String, IndexMap<String, FieldDefault>>) -> Value {
    Value::Object(
        values
            .iter()
            .map(|(key, item)| (key.clone(), object_item_to_json(item)))
            .collect(),
    )
}

fn validate_json_contains(
    actual: &Value,
    expected: &Value,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    match (actual, expected) {
        (Value::Object(actual), Value::Object(expected)) => {
            for (key, expected_value) in expected {
                let path = format!("{path}.{key}");
                match actual.get(key) {
                    Some(actual_value) => {
                        validate_json_contains(actual_value, expected_value, &path, errors);
                    }
                    None => errors.push(ValidationError::new(path, "missing from typed config")),
                }
            }
        }
        (Value::Array(actual), Value::Array(expected)) => {
            if actual.len() != expected.len() {
                errors.push(ValidationError::new(
                    path,
                    format!(
                        "typed config array length {} differs from contract default length {}",
                        actual.len(),
                        expected.len()
                    ),
                ));
                return;
            }
            for (index, (actual_value, expected_value)) in
                actual.iter().zip(expected.iter()).enumerate()
            {
                validate_json_contains(
                    actual_value,
                    expected_value,
                    &format!("{path}[{index}]"),
                    errors,
                );
            }
        }
        (Value::Number(actual), Value::Number(expected))
            if numbers_match(actual.as_f64(), expected.as_f64()) => {}
        _ if actual == expected => {}
        _ => errors.push(ValidationError::new(
            path,
            format!("typed config value {actual:?} differs from contract default {expected:?}"),
        )),
    }
}

fn numbers_match(actual: Option<f64>, expected: Option<f64>) -> bool {
    let (Some(actual), Some(expected)) = (actual, expected) else {
        return false;
    };
    (actual - expected).abs() <= 1e-6
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::parse_spec_str;
    use serde_json::json;

    #[test]
    fn materializes_nested_defaults_by_config_key() {
        let spec = parse_spec_str(
            r##"
schema_version = 1

[field.enabled]
type = "boolean"
default = true

[field.audio_inputs]
type = "string_array"
config_key = "audio.inputs"
default = ["mic", "system"]

[field.color]
type = "color"
config_key = "display.color"
default = "#ffffff"
"##,
        )
        .unwrap();

        assert_eq!(
            defaults_json_from_spec(&spec).unwrap(),
            json!({
                "enabled": true,
                "audio": { "inputs": ["mic", "system"] },
                "display": { "color": "#ffffff" }
            })
        );
    }

    #[test]
    fn typed_defaults_deserialize_integer_numbers() {
        #[derive(Debug, serde::Deserialize, PartialEq)]
        struct Config {
            size: u32,
        }

        let spec = parse_spec_str(
            r#"
schema_version = 1

[field.size]
type = "number"
default = 18
"#,
        )
        .unwrap();

        assert_eq!(
            typed_defaults_from_spec::<Config>(&spec).unwrap(),
            Config { size: 18 }
        );
    }

    #[test]
    fn validates_defaults_are_not_dropped_by_typed_config() {
        #[derive(Debug, serde::Deserialize, serde::Serialize)]
        struct Config {
            audio: Audio,
        }
        #[derive(Debug, serde::Deserialize, serde::Serialize)]
        struct Audio {
            enabled: bool,
        }

        let spec = parse_spec_str(
            r#"
schema_version = 1

[field.audio_enabled]
type = "boolean"
config_key = "audio.enabled"
default = true

[field.audio_inputs]
type = "string_array"
config_key = "audio.inputs"
default = ["mic"]
"#,
        )
        .unwrap();

        let errors = validate_defaults_match_type::<Config>(&spec).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.path == "defaults.audio.inputs"),
            "{errors:?}"
        );
    }

    #[test]
    fn skips_non_stored_fields() {
        let spec = parse_spec_str(
            r#"
schema_version = 1

[field.refresh]
type = "action"
action = "refresh"

[field.status]
type = "status"
query = "status"
"#,
        )
        .unwrap();

        assert_eq!(defaults_json_from_spec(&spec).unwrap(), json!({}));
    }

    #[test]
    fn rejects_config_key_collisions() {
        let spec = parse_spec_str(
            r#"
schema_version = 1

[field.parent]
type = "string"
config_key = "display"
default = "x"

[field.child]
type = "number"
config_key = "display.scale"
default = 1
"#,
        )
        .unwrap();

        let errors = defaults_json_from_spec(&spec).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|error| error.message.contains("collides with field.")),
            "{errors:?}"
        );
    }
}
