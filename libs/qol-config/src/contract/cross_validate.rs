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
        validate_active_action_refs(id, field, runtime, &mut errors);
        validate_stream_ref(id, field, runtime, &mut errors);
        validate_row_action_ref(id, field, runtime, &mut errors);
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn validate_active_action_refs(
    id: &str,
    field: &FieldSpec,
    runtime: Option<&RuntimeSpec>,
    errors: &mut Vec<ValidationError>,
) {
    if field.active_action.is_none()
        && field.active_query.is_none()
        && field.active_value_from.is_none()
    {
        return;
    }
    if field.kind != FieldKind::Action {
        errors.push(ValidationError::new(
            format!("field.{id}"),
            "runtime active state is only valid for action fields",
        ));
        return;
    }
    let Some(action) = field.active_action.as_deref() else {
        errors.push(ValidationError::new(
            format!("field.{id}.active_action"),
            "is required when runtime active state is configured",
        ));
        return;
    };
    let Some(query) = field.active_query.as_deref() else {
        errors.push(ValidationError::new(
            format!("field.{id}.active_query"),
            "is required when active_action is configured",
        ));
        return;
    };
    if field.active_value_from.as_deref().is_none_or(str::is_empty) {
        errors.push(ValidationError::new(
            format!("field.{id}.active_value_from"),
            "is required when active_action is configured",
        ));
        return;
    }
    let Some(runtime) = runtime else {
        errors.push(ValidationError::new(
            format!("field.{id}"),
            "runtime active state requires qol-runtime.toml",
        ));
        return;
    };
    if !runtime.actions.contains_key(action) {
        errors.push(ValidationError::new(
            format!("field.{id}.active_action"),
            format!("references undeclared action: {action}"),
        ));
    }
    if !runtime.queries.contains_key(query) {
        errors.push(ValidationError::new(
            format!("field.{id}.active_query"),
            format!("references undeclared query: {query}"),
        ));
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
        FieldKind::List
        | FieldKind::Status
        | FieldKind::QrCode
        | FieldKind::Gamepad
        | FieldKind::Select
        | FieldKind::StringArray
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
    let actions = field
        .row_action
        .iter()
        .chain(field.row_actions.iter())
        .collect::<Vec<_>>();
    if actions.is_empty() {
        return;
    }
    let Some(rt) = runtime else {
        errors.push(ValidationError::new(
            format!("field.{id}.row_action"),
            "row_action requires qol-runtime.toml but none is present".to_string(),
        ));
        return;
    };
    for (index, action) in actions.into_iter().enumerate() {
        if rt.actions.contains_key(action.action.as_str()) {
            continue;
        }
        let path = if index == 0 && field.row_action.is_some() {
            format!("field.{id}.row_action.action")
        } else {
            format!("field.{id}.row_actions.action")
        };
        errors.push(ValidationError::new(
            path,
            format!("references undeclared action: {}", action.action),
        ));
    }
}

fn runable_reference_for(field: &FieldSpec) -> Option<&str> {
    match field.kind {
        FieldKind::Action => field.action.as_deref(),
        FieldKind::List
        | FieldKind::Status
        | FieldKind::QrCode
        | FieldKind::Gamepad
        | FieldKind::Select
        | FieldKind::StringArray => field.query.as_deref(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::runtime::parse_runtime_spec_str;
    use crate::contract::v1::parse_spec_str;

    #[test]
    fn dynamic_option_queries_must_be_declared_in_runtime() {
        let config = parse_spec_str(
            r#"
schema_version = 1

[field.device]
type = "select"
config_key = "audio.device"
default = "default"
query = "audio_sources"

[field.devices]
type = "string_array"
config_key = "managed_devices"
default = []
query = "device_options"
"#,
        )
        .expect("parse config");
        let runtime = parse_runtime_spec_str(
            r#"
schema_version = 1

[query.other]
description = "something else"
poll_interval_ms = 1000
"#,
        )
        .expect("parse runtime");

        let errors = validate_contracts(&config, Some(&runtime)).expect_err("must fail");
        assert!(
            errors.iter().any(|error| error.path == "field.device.query"
                && error.message.contains("audio_sources")),
            "{errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|error| error.path == "field.devices.query"
                    && error.message.contains("device_options")),
            "{errors:?}"
        );

        let declared = parse_runtime_spec_str(
            r#"
schema_version = 1

[query.audio_sources]
description = "PulseAudio capture sources"
poll_interval_ms = 5000

[query.device_options]
description = "Bluetooth devices"
poll_interval_ms = 5000
"#,
        )
        .expect("parse runtime");
        assert!(validate_contracts(&config, Some(&declared)).is_ok());
    }

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
    fn validates_action_runtime_active_state_references() {
        let config = parse_spec_str(
            r#"
schema_version = 1

[field.search]
type = "action"
action = "start_search"
active_action = "stop_search"
active_query = "search_status"
active_value_from = "searching"
"#,
        )
        .expect("parse config");
        let valid_runtime = parse_runtime_spec_str(
            r#"
schema_version = 1

[action.start_search]
description = "Start searching"

[action.stop_search]
description = "Stop searching"

[query.search_status]
description = "Search status"
poll_interval_ms = 500
"#,
        )
        .expect("parse runtime");
        assert!(validate_contracts(&config, Some(&valid_runtime)).is_ok());

        let invalid_runtime = parse_runtime_spec_str(
            r#"
schema_version = 1

[action.start_search]
description = "Start searching"
"#,
        )
        .expect("parse runtime");
        let errors = validate_contracts(&config, Some(&invalid_runtime)).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.path == "field.search.active_action"));
        assert!(errors
            .iter()
            .any(|error| error.path == "field.search.active_query"));
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
    fn validates_every_state_driven_row_action() {
        let config = parse_spec_str(
            r#"
schema_version = 1

[field.devices]
type = "list"
query = "devices"

[[field.devices.row_actions]]
action = "pair_device"
when = "can_pair"

[[field.devices.row_actions]]
action = "connect_device"
when = "can_connect"
"#,
        )
        .expect("parse config");
        let runtime = parse_runtime_spec_str(
            r#"
schema_version = 1

[query.devices]
description = "Devices"
poll_interval_ms = 1000

[action.pair_device]
description = "Pair"
"#,
        )
        .expect("parse runtime");
        let errors = validate_contracts(&config, Some(&runtime)).unwrap_err();
        assert!(errors.iter().any(|error| {
            error.path == "field.devices.row_actions.action"
                && error.message.contains("connect_device")
        }));
    }

    #[test]
    fn accepts_gamepad_with_native_input_query() {
        let config = parse_spec_str(
            r#"
schema_version = 1

[field.input_test]
type = "gamepad"
query = "controller_input"
"#,
        )
        .expect("parse config");
        let runtime = parse_runtime_spec_str(
            r#"
schema_version = 1

[query.controller_input]
description = "Native controller input supplement"
poll_interval_ms = 32
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
