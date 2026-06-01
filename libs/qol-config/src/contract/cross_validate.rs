use crate::contract::runtime::RuntimeSpec;
use crate::contract::v1::{ConfigSpec, FieldKind, FieldSpec};
use crate::validation::ValidationError;

const STREAMABLE_KINDS: &[FieldKind] = &[FieldKind::Color, FieldKind::Number];

pub fn validate_contracts(
    config: &ConfigSpec,
    runtime: Option<&RuntimeSpec>,
) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    for (id, field) in &config.fields {
        validate_runable_ref(id, field, runtime, &mut errors);
        validate_stream_ref(id, field, runtime, &mut errors);
        validate_row_action_ref(id, field, runtime, &mut errors);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_runable_ref(
    id: &str,
    field: &FieldSpec,
    runtime: Option<&RuntimeSpec>,
    errors: &mut Vec<ValidationError>,
) {
    let Some(ref_name) = runable_reference_for(field) else {
        return;
    };
    let Some(rt) = runtime else {
        errors.push(ValidationError::new(
            format!("field.{id}"),
            format!(
                "kind {:?} requires qol-runtime.toml but none is present",
                field.kind
            ),
        ));
        return;
    };
    match field.kind {
        FieldKind::Action if !rt.actions.contains_key(ref_name) => {
            errors.push(ValidationError::new(
                format!("field.{id}.action"),
                format!("references undeclared action: {ref_name}"),
            ));
        }
        FieldKind::List | FieldKind::Status | FieldKind::QrCode
            if !rt.queries.contains_key(ref_name) =>
        {
            errors.push(ValidationError::new(
                format!("field.{id}.query"),
                format!("references undeclared query: {ref_name}"),
            ));
        }
        _ => {}
    }
}

fn validate_stream_ref(
    id: &str,
    field: &FieldSpec,
    runtime: Option<&RuntimeSpec>,
    errors: &mut Vec<ValidationError>,
) {
    let Some(ref stream_name) = field.stream else {
        return;
    };
    if !STREAMABLE_KINDS.contains(&field.kind) {
        errors.push(ValidationError::new(
            format!("field.{id}.stream"),
            format!("kind {:?} does not support streams", field.kind),
        ));
        return;
    }
    let Some(rt) = runtime else {
        errors.push(ValidationError::new(
            format!("field.{id}.stream"),
            "stream requires qol-runtime.toml but none is present".to_string(),
        ));
        return;
    };
    if !rt.streams.contains_key(stream_name.as_str()) {
        errors.push(ValidationError::new(
            format!("field.{id}.stream"),
            format!("references undeclared stream: {stream_name}"),
        ));
    }
}

fn validate_row_action_ref(
    id: &str,
    field: &FieldSpec,
    runtime: Option<&RuntimeSpec>,
    errors: &mut Vec<ValidationError>,
) {
    let Some(ref ra) = field.row_action else {
        return;
    };
    let Some(rt) = runtime else {
        errors.push(ValidationError::new(
            format!("field.{id}.row_action"),
            "row_action requires qol-runtime.toml but none is present".to_string(),
        ));
        return;
    };
    if !rt.actions.contains_key(ra.action.as_str()) {
        errors.push(ValidationError::new(
            format!("field.{id}.row_action.action"),
            format!("references undeclared action: {}", ra.action),
        ));
    }
}

fn runable_reference_for(field: &FieldSpec) -> Option<&str> {
    match field.kind {
        FieldKind::Action => field.action.as_deref(),
        FieldKind::List | FieldKind::Status | FieldKind::QrCode => field.query.as_deref(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::runtime::parse_runtime_spec_str;
    use crate::contract::v1::parse_spec_str;

    #[test]
    fn accepts_consistent_contracts() {
        let config = parse_spec_str(
            r#"
schema_version = 1

[field.pair_btn]
type = "action"
label = "Pair"
action = "pair_device"
"#,
        )
        .expect("parse config");
        let runtime = parse_runtime_spec_str(
            r#"
schema_version = 1

[action.pair_device]
description = "Pair device"
"#,
        )
        .expect("parse runtime");
        assert!(
            validate_contracts(&config, Some(&runtime)).is_ok(),
            "consistent contracts should validate"
        );
    }

    #[test]
    fn rejects_dangling_action_reference() {
        let config = parse_spec_str(
            r#"
schema_version = 1

[field.pair_btn]
type = "action"
label = "Pair"
action = "pair_device"
"#,
        )
        .expect("parse config");
        let runtime = parse_runtime_spec_str("schema_version = 1\n").expect("parse runtime");
        let result = validate_contracts(&config, Some(&runtime));
        assert!(result.is_err(), "dangling action reference should fail");
        let errors = result.unwrap_err();
        assert!(
            errors.iter().any(|e| e.to_string().contains("pair_device")),
            "error should mention the dangling reference, got: {:?}",
            errors
        );
    }

    #[test]
    fn rejects_runable_field_without_runtime_spec() {
        let config = parse_spec_str(
            r#"
schema_version = 1

[field.pair_btn]
type = "action"
label = "Pair"
action = "pair_device"
"#,
        )
        .expect("parse config");
        let result = validate_contracts(&config, None);
        assert!(result.is_err(), "action field without runtime should fail");
    }

    #[test]
    fn accepts_color_field_with_stream() {
        let config = parse_spec_str(
            r#"
schema_version = 1

[field.color]
type = "color"
stream = "live_color"
"#,
        )
        .expect("parse config");
        let runtime = parse_runtime_spec_str(
            r#"
schema_version = 1

[stream.live_color]
description = "Color control"
throttle_ms = 100
"#,
        )
        .expect("parse runtime");
        assert!(validate_contracts(&config, Some(&runtime)).is_ok());
    }

    #[test]
    fn rejects_dangling_stream_reference() {
        let config = parse_spec_str(
            r#"
schema_version = 1

[field.color]
type = "color"
stream = "nonexistent"
"#,
        )
        .expect("parse config");
        let runtime = parse_runtime_spec_str("schema_version = 1\n").expect("parse runtime");
        let result = validate_contracts(&config, Some(&runtime));
        assert!(result.is_err(), "dangling stream should fail");
    }

    #[test]
    fn rejects_stream_on_non_streamable_kind() {
        let config = parse_spec_str(
            r#"
schema_version = 1

[field.name]
type = "string"
stream = "live_name"
"#,
        )
        .expect("parse config");
        let runtime = parse_runtime_spec_str(
            r#"
schema_version = 1

[stream.live_name]
description = "test"
throttle_ms = 100
"#,
        )
        .expect("parse runtime");
        let result = validate_contracts(&config, Some(&runtime));
        assert!(result.is_err(), "string field with stream should fail");
    }

    #[test]
    fn accepts_list_with_row_action() {
        let config = parse_spec_str(
            r#"
schema_version = 1

[field.devices]
type = "list"
query = "list_devices"
row_label = "{name}"

[field.devices.row_action]
action = "remove_device"
label = "Remove"
key = "Delete"
"#,
        )
        .expect("parse config");
        let runtime = parse_runtime_spec_str(
            r#"
schema_version = 1

[query.list_devices]
description = "List devices"
poll_interval_ms = 2000

[action.remove_device]
description = "Remove a device"
"#,
        )
        .expect("parse runtime");
        assert!(validate_contracts(&config, Some(&runtime)).is_ok());
    }

    #[test]
    fn rejects_row_action_with_dangling_action() {
        let config = parse_spec_str(
            r#"
schema_version = 1

[field.devices]
type = "list"
query = "list_devices"

[field.devices.row_action]
action = "nonexistent"
"#,
        )
        .expect("parse config");
        let runtime = parse_runtime_spec_str(
            r#"
schema_version = 1

[query.list_devices]
description = "test"
poll_interval_ms = 1000
"#,
        )
        .expect("parse runtime");
        let result = validate_contracts(&config, Some(&runtime));
        assert!(result.is_err(), "dangling row_action should fail");
    }
}
