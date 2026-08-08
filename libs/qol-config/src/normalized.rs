use crate::contract::{
    ConfigSpec, FieldAlign, FieldDefault, FieldKind, ItemSpec, NumberConstraints, RowActionSpec,
};
use crate::validation::{validate_spec_collect, ValidationError};
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpha: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_value_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_action: Option<RowActionSpec>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub row_actions: Vec<RowActionSpec>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub empty_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_map: Option<IndexMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tone_map: Option<IndexMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<FieldAlign>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span: Option<u8>,
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
        let no_stored_value = !field.kind.has_stored_value();
        let default = widen_to_kind(
            field
                .default
                .clone()
                .unwrap_or(FieldDefault::String(String::new())),
            field.kind,
        );
        let value = if no_stored_value {
            default.clone()
        } else {
            widen_to_kind(
                resolve_field_value(id, field, &default, overrides, &mut errors),
                field.kind,
            )
        };
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
            alpha: field.alpha,
            action: field.action.clone(),
            active_action: field.active_action.clone(),
            active_query: field.active_query.clone(),
            active_value_from: field.active_value_from.clone(),
            active_label: field.active_label.clone(),
            variant: field.variant.clone(),
            query: field.query.clone(),
            stream: field.stream.clone(),
            row_label: field.row_label.clone(),
            row_subtitle: field.row_subtitle.clone(),
            row_action: field.row_action.clone(),
            row_actions: field.row_actions.clone(),
            search: field.search,
            empty_message: field.empty_message.clone(),
            value_from: field.value_from.clone(),
            label_map: field.label_map.clone(),
            tone_map: field.tone_map.clone(),
            align: field.align,
            span: field.span,
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

pub fn widen_to_kind(value: FieldDefault, kind: FieldKind) -> FieldDefault {
    match (kind, &value) {
        (FieldKind::Number, FieldDefault::String(text)) => match text.trim().parse::<f64>() {
            Ok(number) if number.is_finite() => FieldDefault::Number(number),
            _ => FieldDefault::String(text.clone()),
        },
        (FieldKind::Boolean, FieldDefault::String(text)) => match text.as_str() {
            "true" => FieldDefault::Boolean(true),
            "false" => FieldDefault::Boolean(false),
            _ => FieldDefault::String(text.clone()),
        },
        (FieldKind::Boolean, FieldDefault::Number(number)) => match number {
            1.0 => FieldDefault::Boolean(true),
            0.0 => FieldDefault::Boolean(false),
            _ => FieldDefault::Number(*number),
        },
        (
            FieldKind::String | FieldKind::Select | FieldKind::Color,
            FieldDefault::Number(number),
        ) => FieldDefault::String(format!("{number}")),
        (FieldKind::String | FieldKind::Select | FieldKind::Color, FieldDefault::Boolean(flag)) => {
            FieldDefault::String(flag.to_string())
        }
        (FieldKind::StringArray, FieldDefault::String(text)) => {
            FieldDefault::StringArray(vec![text.clone()])
        }
        (FieldKind::ObjectArray, FieldDefault::StringArray(values)) if values.is_empty() => {
            FieldDefault::ObjectArray(Vec::new())
        }
        _ => value,
    }
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
    let value = match field_default_from_override(raw) {
        Some(value) => value,
        None => {
            errors.push(ValidationError::new(
                format!("overrides.{id}"),
                format!("value does not match field type {}", field.kind.name()),
            ));
            return default.clone();
        }
    };
    value
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

pub(crate) fn field_default_from_override(raw: &serde_json::Value) -> Option<FieldDefault> {
    if let Some(flag) = raw.as_bool() {
        return Some(FieldDefault::Boolean(flag));
    }
    if let Some(number) = raw.as_f64() {
        return Some(FieldDefault::Number(number));
    }
    if let Some(text) = raw.as_str() {
        return Some(FieldDefault::String(text.to_string()));
    }
    if let Some(values) = raw.as_array() {
        if let Some(strings) = string_array_from_json(values) {
            return Some(FieldDefault::StringArray(strings));
        }
        return object_array_from_json(values).map(FieldDefault::ObjectArray);
    }
    object_map_from_json(raw.as_object()?).map(FieldDefault::ObjectMap)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::parse_spec_str;

    const FREE_FORM_SPEC: &str = r#"
schema_version = 1

[field.service_commands]
type = "string_array"
config_key = "service_commands"
default = []
"#;

    #[test]
    fn free_form_string_array_normalizes_overrides_and_defaults() {
        let spec = parse_spec_str(FREE_FORM_SPEC).unwrap();
        let resolved = resolve_config(
            &spec,
            &serde_json::json!({ "service_commands": ["cargo watch", "tail -f"] }),
        )
        .unwrap();
        let field = &resolved.fields[0];
        assert_eq!(field.kind, FieldKind::StringArray);
        assert_eq!(
            field.value,
            FieldDefault::StringArray(vec!["cargo watch".to_string(), "tail -f".to_string()])
        );
        assert_eq!(field.default, FieldDefault::StringArray(Vec::new()));
    }

    #[test]
    fn free_form_string_array_scalar_override_widens_to_a_single_item_list() {
        let spec = parse_spec_str(FREE_FORM_SPEC).unwrap();
        let resolved = resolve_config(
            &spec,
            &serde_json::json!({ "service_commands": "cargo watch" }),
        )
        .unwrap();
        let field = &resolved.fields[0];
        assert_eq!(
            field.value,
            FieldDefault::StringArray(vec!["cargo watch".to_string()]),
            "a scalar override widens to a single-item list so the editor stays usable"
        );
    }

    #[test]
    fn an_empty_object_array_default_keeps_its_declared_kind() {
        let spec = parse_spec_str(
            r#"
schema_version = 1

[field.switchable_panels]
type = "object_array"
default = []

[field.switchable_panels.item.fields]
app = "string"
switchable = "boolean"
"#,
        )
        .unwrap();

        for (label, overrides) in [
            ("contract default", serde_json::json!({})),
            (
                "empty override",
                serde_json::json!({ "switchable_panels": [] }),
            ),
        ] {
            let resolved = resolve_config(&spec, &overrides).unwrap();
            assert_eq!(
                resolved.fields[0].value,
                FieldDefault::ObjectArray(Vec::new()),
                "{label} must stay an object array so renderers can build an editor"
            );
            assert_eq!(
                resolved.fields[0].default,
                FieldDefault::ObjectArray(Vec::new())
            );
        }
    }
}
