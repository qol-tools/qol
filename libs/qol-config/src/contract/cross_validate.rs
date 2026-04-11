use crate::contract::runtime::RuntimeSpec;
use crate::contract::v1::{ConfigSpec, FieldKind, FieldSpec};
use crate::validation::ValidationError;

pub fn validate_contracts(
    config: &ConfigSpec,
    runtime: Option<&RuntimeSpec>,
) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    for (id, field) in &config.fields {
        let Some(runable_ref) = runable_reference_for(field) else {
            continue;
        };
        let runtime = match runtime {
            Some(r) => r,
            None => {
                errors.push(ValidationError::new(
                    format!("field.{id}"),
                    format!(
                        "field uses kind {:?} which requires qol-runtime.toml but none is present",
                        field.kind
                    ),
                ));
                continue;
            }
        };
        match field.kind {
            FieldKind::Action => {
                if !runtime.actions.contains_key(runable_ref) {
                    errors.push(ValidationError::new(
                        format!("field.{id}.action"),
                        format!("references undeclared action: {runable_ref}"),
                    ));
                }
            }
            FieldKind::List | FieldKind::Status | FieldKind::QrCode => {
                if !runtime.queries.contains_key(runable_ref) {
                    errors.push(ValidationError::new(
                        format!("field.{id}.query"),
                        format!("references undeclared query: {runable_ref}"),
                    ));
                }
            }
            FieldKind::Boolean
            | FieldKind::String
            | FieldKind::Number
            | FieldKind::Select
            | FieldKind::StringArray
            | FieldKind::ObjectArray
            | FieldKind::ObjectMap
            | FieldKind::Color => {}
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn runable_reference_for(field: &FieldSpec) -> Option<&str> {
    match field.kind {
        FieldKind::Action => field.action.as_deref(),
        FieldKind::List | FieldKind::Status | FieldKind::QrCode => field.query.as_deref(),
        FieldKind::Boolean
        | FieldKind::String
        | FieldKind::Number
        | FieldKind::Select
        | FieldKind::StringArray
        | FieldKind::ObjectArray
        | FieldKind::ObjectMap
        | FieldKind::Color => None,
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
}
