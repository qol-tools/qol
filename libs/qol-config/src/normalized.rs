use crate::contract::{ConfigSpec, FieldDefault, FieldKind, ItemSpec, NumberConstraints};
use crate::validation::{validate_field_value, validate_spec_collect, ValidationError};
use indexmap::IndexMap;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResolvedConfig {
    pub title: Option<String>,
    pub description: Option<String>,
    pub fields: Vec<ResolvedField>,
    pub sections: Vec<ResolvedSection>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResolvedSection {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub actions: Vec<String>,
    pub fields: Vec<ResolvedField>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResolvedField {
    pub id: String,
    pub kind: FieldKind,
    pub config_key: String,
    pub label: String,
    pub description: Option<String>,
    pub placeholder: Option<String>,
    pub value: FieldDefault,
    pub default: FieldDefault,
    pub options: Vec<String>,
    pub option_labels: std::collections::BTreeMap<String, String>,
    pub key_label: Option<String>,
    pub entry_fields: std::collections::BTreeMap<String, FieldKind>,
    pub item: Option<ResolvedItemSpec>,
    pub show_when: Option<ResolvedShowWhen>,
    pub number: NumberConstraints,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResolvedItemSpec {
    pub fields: std::collections::BTreeMap<String, FieldKind>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ResolvedShowWhen {
    pub field: String,
    pub equals: FieldDefault,
}

pub fn resolve_config(
    spec: &ConfigSpec,
    overrides: &serde_json::Value,
) -> Result<ResolvedConfig, Vec<ValidationError>> {
    let mut errors = validate_spec_collect(spec);
    validate_overrides_shape(overrides, &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }

    let mut root_fields = Vec::new();
    let mut sections = build_sections(spec);

    for (id, field) in &spec.fields {
        let default = field.default.clone().expect("validated default");
        let value = resolve_field_value(id, field, &default, overrides, &mut errors);
        let resolved = ResolvedField {
            id: id.clone(),
            kind: field.kind,
            config_key: config_key_for(id, field),
            label: field.label.clone().unwrap_or_else(|| humanize(id)),
            description: field.description.clone(),
            placeholder: field.placeholder.clone(),
            value,
            default,
            options: field.options.clone(),
            option_labels: field
                .option_labels
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            key_label: field.key_label.clone(),
            entry_fields: field
                .entry_fields
                .iter()
                .map(|(key, value)| (key.clone(), *value))
                .collect(),
            item: resolve_item_spec(field.item.as_ref()),
            show_when: field.show_when.as_ref().map(|show_when| ResolvedShowWhen {
                field: show_when.field.clone(),
                equals: show_when.equals.clone(),
            }),
            number: field.number.clone(),
        };
        push_resolved_field(
            &mut root_fields,
            &mut sections,
            field.section.as_deref(),
            resolved,
        );
    }

    Ok(ResolvedConfig {
        title: spec.title.clone(),
        description: spec.description.clone(),
        fields: root_fields,
        sections,
    })
}

fn resolve_item_spec(item: Option<&ItemSpec>) -> Option<ResolvedItemSpec> {
    let item = item?;
    if item.fields.is_empty() {
        return None;
    }
    Some(ResolvedItemSpec {
        fields: item.fields.iter().map(|(k, v)| (k.clone(), *v)).collect(),
    })
}

fn validate_overrides_shape(overrides: &serde_json::Value, errors: &mut Vec<ValidationError>) {
    if overrides.is_null() || overrides.is_object() {
        return;
    }
    errors.push(ValidationError::new("overrides", "must be a JSON object"));
}

fn build_sections(spec: &ConfigSpec) -> Vec<ResolvedSection> {
    spec.sections
        .iter()
        .map(|(id, section)| ResolvedSection {
            id: id.clone(),
            label: section.label.clone().unwrap_or_else(|| humanize(id)),
            description: section.description.clone(),
            actions: section.actions.clone(),
            fields: Vec::new(),
        })
        .collect()
}

fn resolve_field_value(
    id: &str,
    field: &crate::contract::FieldSpec,
    default: &FieldDefault,
    overrides: &serde_json::Value,
    errors: &mut Vec<ValidationError>,
) -> FieldDefault {
    let config_key = config_key_for(id, field);
    let raw = match get_override_value(overrides, &config_key) {
        Some(raw) => raw,
        None => return default.clone(),
    };
    let value = match field_default_from_override(field.kind, raw) {
        Some(value) => value,
        None => {
            errors.push(ValidationError::new(
                format!("overrides.{id}"),
                format!(
                    "value does not match field type {}",
                    field_kind_name(field.kind)
                ),
            ));
            return default.clone();
        }
    };
    let validation_errors = validate_field_value(&format!("overrides.{id}"), field, &value);
    if validation_errors.is_empty() {
        return value;
    }
    errors.extend(validation_errors);
    default.clone()
}

fn get_override_value<'a>(
    overrides: &'a serde_json::Value,
    path: &str,
) -> Option<&'a serde_json::Value> {
    let mut current = overrides;
    for part in path.split('.') {
        current = current.get(part)?;
    }
    Some(current)
}

fn config_key_for(id: &str, field: &crate::contract::FieldSpec) -> String {
    field.config_key.clone().unwrap_or_else(|| id.to_string())
}

fn field_default_from_override(kind: FieldKind, raw: &serde_json::Value) -> Option<FieldDefault> {
    match kind {
        FieldKind::Boolean => raw.as_bool().map(FieldDefault::Boolean),
        FieldKind::String | FieldKind::Select => raw
            .as_str()
            .map(|value| FieldDefault::String(value.to_string())),
        FieldKind::Number => raw.as_f64().map(FieldDefault::Number),
        FieldKind::StringArray => {
            let values = raw.as_array()?;
            string_array_from_json(values).map(FieldDefault::StringArray)
        }
        FieldKind::ObjectArray => {
            let values = raw.as_array()?;
            object_array_from_json(values).map(FieldDefault::ObjectArray)
        }
        FieldKind::ObjectMap => {
            let values = raw.as_object()?;
            object_map_from_json(values).map(FieldDefault::ObjectMap)
        }
    }
}

fn string_array_from_json(values: &[serde_json::Value]) -> Option<Vec<String>> {
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        result.push(value.as_str()?.to_string());
    }
    Some(result)
}

fn object_array_from_json(
    values: &[serde_json::Value],
) -> Option<Vec<IndexMap<String, FieldDefault>>> {
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        result.push(object_item_from_json(value)?);
    }
    Some(result)
}

fn object_item_from_json(value: &serde_json::Value) -> Option<IndexMap<String, FieldDefault>> {
    let object = value.as_object()?;
    let mut result = IndexMap::new();
    for (key, value) in object {
        result.insert(key.clone(), object_field_value_from_json(value)?);
    }
    Some(result)
}

fn object_field_value_from_json(value: &serde_json::Value) -> Option<FieldDefault> {
    if let Some(boolean) = value.as_bool() {
        return Some(FieldDefault::Boolean(boolean));
    }
    if let Some(number) = value.as_f64() {
        return Some(FieldDefault::Number(number));
    }
    if let Some(text) = value.as_str() {
        return Some(FieldDefault::String(text.to_string()));
    }
    let values = value.as_array()?;
    string_array_from_json(values).map(FieldDefault::StringArray)
}

fn object_map_from_json(
    values: &serde_json::Map<String, serde_json::Value>,
) -> Option<IndexMap<String, IndexMap<String, FieldDefault>>> {
    let mut result = IndexMap::new();
    for (key, value) in values {
        result.insert(key.clone(), object_item_from_json(value)?);
    }
    Some(result)
}

fn push_resolved_field(
    root_fields: &mut Vec<ResolvedField>,
    sections: &mut [ResolvedSection],
    section_id: Option<&str>,
    field: ResolvedField,
) {
    let section_id = match section_id {
        Some(section_id) => section_id,
        None => {
            root_fields.push(field);
            return;
        }
    };
    if let Some(section) = sections.iter_mut().find(|section| section.id == section_id) {
        section.fields.push(field);
    }
}

fn humanize(value: &str) -> String {
    value
        .split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(title_case)
        .collect::<Vec<_>>()
        .join(" ")
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    let first = match chars.next() {
        Some(first) => first,
        None => return String::new(),
    };
    let mut result = String::new();
    result.extend(first.to_uppercase());
    result.push_str(chars.as_str());
    result
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
