use crate::contract::{ConfigSpec, FieldDefault, FieldKind, FieldSpec};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub path: String,
    pub message: String,
}

impl ValidationError {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

pub fn validate_spec(spec: &ConfigSpec) -> Result<(), Vec<ValidationError>> {
    let errors = validate_spec_collect(spec);
    if errors.is_empty() {
        return Ok(());
    }
    Err(errors)
}

pub fn validate_spec_collect(spec: &ConfigSpec) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    validate_schema_version(spec, &mut errors);
    validate_sections(spec, &mut errors);
    validate_fields(spec, &mut errors);
    errors
}

pub fn validate_field_value(
    path: &str,
    field: &FieldSpec,
    value: &FieldDefault,
) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    validate_field_value_type(path, field, value, &mut errors);
    validate_select_value(path, field, value, &mut errors);
    validate_number_value(path, field, value, &mut errors);
    errors
}

fn validate_schema_version(spec: &ConfigSpec, errors: &mut Vec<ValidationError>) {
    if spec.schema_version == 1 {
        return;
    }
    errors.push(ValidationError::new(
        "schema_version",
        format!("unsupported version {}", spec.schema_version),
    ));
}

fn validate_sections(spec: &ConfigSpec, errors: &mut Vec<ValidationError>) {
    for (id, section) in &spec.sections {
        if valid_id(id) {
            validate_section_actions(id, &section.actions, errors);
            continue;
        }
        errors.push(ValidationError::new(format!("section.{id}"), "invalid id"));
        validate_section_actions(id, &section.actions, errors);
    }
}

fn validate_section_actions(id: &str, actions: &[String], errors: &mut Vec<ValidationError>) {
    let mut seen = std::collections::HashSet::new();
    for (index, action) in actions.iter().enumerate() {
        if !valid_id(action) {
            errors.push(ValidationError::new(
                format!("section.{id}.actions[{index}]"),
                "invalid action id",
            ));
            continue;
        }
        if seen.insert(action) {
            continue;
        }
        errors.push(ValidationError::new(
            format!("section.{id}.actions[{index}]"),
            "duplicate action id",
        ));
    }
}

fn validate_fields(spec: &ConfigSpec, errors: &mut Vec<ValidationError>) {
    for (id, field) in &spec.fields {
        validate_field_id(id, errors);
        validate_field_config_key(id, field, errors);
        validate_field_section(id, field.section.as_deref(), spec, errors);
        validate_field_default(id, field, errors);
        validate_field_options(id, field, errors);
        validate_entry_fields(id, field, errors);
        validate_number_constraints(id, field, errors);
        validate_show_when(id, field, spec, errors);
    }
}

fn validate_field_id(id: &str, errors: &mut Vec<ValidationError>) {
    if valid_id(id) {
        return;
    }
    errors.push(ValidationError::new(format!("field.{id}"), "invalid id"));
}

fn validate_field_config_key(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    let Some(config_key) = field.config_key.as_deref() else {
        return;
    };
    if valid_config_key(config_key) {
        return;
    }
    errors.push(ValidationError::new(
        format!("field.{id}.config_key"),
        "invalid config key",
    ));
}

fn validate_field_section(
    id: &str,
    section: Option<&str>,
    spec: &ConfigSpec,
    errors: &mut Vec<ValidationError>,
) {
    let section = match section {
        Some(section) => section,
        None => return,
    };
    if spec.sections.contains_key(section) {
        return;
    }
    errors.push(ValidationError::new(
        format!("field.{id}.section"),
        format!("unknown section {section}"),
    ));
}

fn validate_field_default(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    let default = match &field.default {
        Some(default) => default,
        None => {
            errors.push(ValidationError::new(
                format!("field.{id}.default"),
                "missing default",
            ));
            return;
        }
    };
    errors.extend(validate_field_value(
        &format!("field.{id}.default"),
        field,
        default,
    ));
}

fn validate_field_options(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    if field.kind == FieldKind::Select {
        validate_select_options(id, field, errors);
        validate_option_labels(id, field, errors);
        return;
    }
    if field.options.is_empty() && field.option_labels.is_empty() {
        return;
    }
    if !field.option_labels.is_empty() {
        errors.push(ValidationError::new(
            format!("field.{id}.option_labels"),
            "option_labels only supported for select fields",
        ));
    }
    if field.options.is_empty() {
        return;
    }
    errors.push(ValidationError::new(
        format!("field.{id}.options"),
        "options only supported for select fields",
    ));
}

fn validate_entry_fields(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    if field.kind == FieldKind::ObjectMap {
        validate_object_map_fields(id, field, errors);
        return;
    }
    if field.key_label.is_none() && field.entry_fields.is_empty() {
        return;
    }
    if field.key_label.is_some() {
        errors.push(ValidationError::new(
            format!("field.{id}.key_label"),
            "key_label only supported for object_map fields",
        ));
    }
    if field.entry_fields.is_empty() {
        return;
    }
    errors.push(ValidationError::new(
        format!("field.{id}.entry_fields"),
        "entry_fields only supported for object_map fields",
    ));
}

fn validate_object_map_fields(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    for entry_id in field.entry_fields.keys() {
        if valid_id(entry_id) {
            continue;
        }
        errors.push(ValidationError::new(
            format!("field.{id}.entry_fields.{entry_id}"),
            "invalid entry field id",
        ));
    }
}

fn validate_select_options(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    if field.options.is_empty() {
        errors.push(ValidationError::new(
            format!("field.{id}.options"),
            "select field requires options",
        ));
        return;
    }
    for (index, option) in field.options.iter().enumerate() {
        if !option.trim().is_empty() {
            continue;
        }
        errors.push(ValidationError::new(
            format!("field.{id}.options[{index}]"),
            "option cannot be empty",
        ));
    }
}

fn validate_option_labels(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    for (option, label) in &field.option_labels {
        if field.options.iter().any(|candidate| candidate == option) {
            if !label.trim().is_empty() {
                continue;
            }
            errors.push(ValidationError::new(
                format!("field.{id}.option_labels.{option}"),
                "option label cannot be empty",
            ));
            continue;
        }
        errors.push(ValidationError::new(
            format!("field.{id}.option_labels.{option}"),
            "option label must reference an existing option",
        ));
    }
}

fn validate_number_constraints(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    if field.kind != FieldKind::Number {
        validate_non_number_constraints(id, field, errors);
        return;
    }
    if let (Some(min), Some(max)) = (field.number.min, field.number.max) {
        if min > max {
            errors.push(ValidationError::new(
                format!("field.{id}"),
                "min cannot be greater than max",
            ));
        }
    }
    validate_number_step(id, field.number.step, errors);
}

fn validate_non_number_constraints(id: &str, field: &FieldSpec, errors: &mut Vec<ValidationError>) {
    if field.number.min.is_none() && field.number.max.is_none() && field.number.step.is_none() {
        return;
    }
    errors.push(ValidationError::new(
        format!("field.{id}"),
        "min, max, and step are only supported for number fields",
    ));
}

fn validate_number_step(id: &str, step: Option<f64>, errors: &mut Vec<ValidationError>) {
    let step = match step {
        Some(step) => step,
        None => return,
    };
    if step > 0.0 {
        return;
    }
    errors.push(ValidationError::new(
        format!("field.{id}.step"),
        "step must be greater than 0",
    ));
}

fn validate_show_when(
    id: &str,
    field: &FieldSpec,
    spec: &ConfigSpec,
    errors: &mut Vec<ValidationError>,
) {
    let show_when = match &field.show_when {
        Some(show_when) => show_when,
        None => return,
    };
    let referenced = match spec.fields.get(show_when.field.as_str()) {
        Some(referenced) => referenced,
        None => {
            errors.push(ValidationError::new(
                format!("field.{id}.show_when.field"),
                format!("unknown field {}", show_when.field),
            ));
            return;
        }
    };
    if default_matches_kind(&show_when.equals, referenced.kind) {
        validate_select_value(
            &format!("field.{id}.show_when.equals"),
            referenced,
            &show_when.equals,
            errors,
        );
        validate_number_value(
            &format!("field.{id}.show_when.equals"),
            referenced,
            &show_when.equals,
            errors,
        );
        return;
    }
    errors.push(ValidationError::new(
        format!("field.{id}.show_when.equals"),
        format!(
            "value does not match field type {}",
            field_kind_name(referenced.kind)
        ),
    ));
}

fn validate_field_value_type(
    path: &str,
    field: &FieldSpec,
    value: &FieldDefault,
    errors: &mut Vec<ValidationError>,
) {
    if default_matches_kind(value, field.kind) {
        return;
    }
    errors.push(ValidationError::new(
        path,
        format!(
            "value does not match field type {}",
            field_kind_name(field.kind)
        ),
    ));
}

fn validate_select_value(
    path: &str,
    field: &FieldSpec,
    value: &FieldDefault,
    errors: &mut Vec<ValidationError>,
) {
    if field.kind != FieldKind::Select {
        return;
    }
    if field.options.is_empty() {
        return;
    }
    let selected = match value {
        FieldDefault::String(selected) => selected,
        _ => return,
    };
    if field.options.iter().any(|option| option == selected) {
        return;
    }
    errors.push(ValidationError::new(
        path,
        "value must match one of the select options",
    ));
}

fn validate_number_value(
    path: &str,
    field: &FieldSpec,
    value: &FieldDefault,
    errors: &mut Vec<ValidationError>,
) {
    if field.kind != FieldKind::Number {
        return;
    }
    let number = match value {
        FieldDefault::Number(number) => *number,
        _ => return,
    };
    validate_min(path, field.number.min, number, errors);
    validate_max(path, field.number.max, number, errors);
    validate_step_value(path, field, number, errors);
}

fn validate_min(path: &str, min: Option<f64>, value: f64, errors: &mut Vec<ValidationError>) {
    let min = match min {
        Some(min) => min,
        None => return,
    };
    if value >= min {
        return;
    }
    errors.push(ValidationError::new(
        path,
        format!("value must be at least {min}"),
    ));
}

fn validate_max(path: &str, max: Option<f64>, value: f64, errors: &mut Vec<ValidationError>) {
    let max = match max {
        Some(max) => max,
        None => return,
    };
    if value <= max {
        return;
    }
    errors.push(ValidationError::new(
        path,
        format!("value must be at most {max}"),
    ));
}

fn validate_step_value(
    path: &str,
    field: &FieldSpec,
    value: f64,
    errors: &mut Vec<ValidationError>,
) {
    let step = match field.number.step {
        Some(step) => step,
        None => return,
    };
    let origin = field.number.min.unwrap_or(0.0);
    let steps = (value - origin) / step;
    let rounded = steps.round();
    if (steps - rounded).abs() <= 1e-9 {
        return;
    }
    errors.push(ValidationError::new(
        path,
        format!("value must align to step {step}"),
    ));
}

fn default_matches_kind(default: &FieldDefault, kind: FieldKind) -> bool {
    match (default, kind) {
        (FieldDefault::Boolean(_), FieldKind::Boolean) => true,
        (FieldDefault::String(_), FieldKind::String | FieldKind::Select) => true,
        (FieldDefault::Number(_), FieldKind::Number) => true,
        (FieldDefault::StringArray(_), FieldKind::StringArray) => true,
        (FieldDefault::ObjectArray(_), FieldKind::ObjectArray) => true,
        (FieldDefault::StringArray(values), FieldKind::ObjectArray) => values.is_empty(),
        (FieldDefault::ObjectMap(_), FieldKind::ObjectMap) => true,
        _ => false,
    }
}

fn valid_id(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 {
        return false;
    }
    value
        .chars()
        .all(|char| char.is_ascii_alphanumeric() || char == '_' || char == '-')
}

fn valid_config_key(value: &str) -> bool {
    if value.is_empty() || value.len() > 128 {
        return false;
    }
    value.split('.').all(valid_id)
}

fn field_kind_name(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Boolean => "boolean",
        FieldKind::String => "string",
        FieldKind::Number => "number",
        FieldKind::Select => "select",
        FieldKind::StringArray => "string_array",
        FieldKind::ObjectArray => "object_array",
        FieldKind::ObjectMap => "object_map",
    }
}
