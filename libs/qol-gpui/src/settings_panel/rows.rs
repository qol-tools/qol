use qol_config::contract::{
    resolve_row_actions, FieldDefault, FieldKind, ResolvedRowAction, RowActionSpec,
};
use qol_config::normalized::{ResolvedConfig, ResolvedField, ResolvedSection, ResolvedShowWhen};
use qol_config::object_array::item_schema;

use super::object_array_row::ObjectArrayState;
use crate::gamepad::GamepadMonitor;
use crate::scroll_list::ScrollList;
use crate::status_indicator::StatusTone;

pub(super) const LIST_MAX_VISIBLE: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ListItem {
    pub(super) id: String,
    pub(super) label: String,
    pub(super) subtitle: Option<String>,
    pub(super) accent: Option<StatusTone>,
    pub(super) badge: Option<String>,
    pub(super) badge_tone: Option<StatusTone>,
    pub(super) data: serde_json::Value,
    pub(super) pending: bool,
    pub(super) error: Option<String>,
}

impl ListItem {
    pub(super) fn effective_badge_tone(&self) -> StatusTone {
        self.badge_tone.or(self.accent).unwrap_or(StatusTone::Muted)
    }
}

#[derive(Debug)]
pub(super) struct ListActions {
    pub(super) primary: Option<RowActionSpec>,
    pub(super) additional: Vec<RowActionSpec>,
}

#[derive(Debug)]
pub(super) struct OptionQuery {
    name: String,
    seeded: Vec<SelectOption>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SelectOption {
    pub(super) value: String,
    pub(super) label: String,
    pub(super) accent: Option<u32>,
}

impl SelectOption {
    pub(super) fn plain(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            accent: None,
        }
    }
}

#[derive(Debug)]
pub(super) enum RowControl {
    Toggle(bool),
    Select {
        options: Vec<SelectOption>,
        index: usize,
        dynamic: Option<OptionQuery>,
    },
    MultiSelect {
        options: Vec<SelectOption>,
        selected: Vec<bool>,
        dynamic: Option<OptionQuery>,
    },
    Number {
        value: f64,
        min: Option<f64>,
        max: Option<f64>,
        step: Option<f64>,
    },
    Text(String),
    TextList(Vec<String>),
    Color(String),
    Action {
        action: String,
        active_action: Option<String>,
        active_label: Option<String>,
        active_query: Option<String>,
        active_value_from: Option<String>,
        state_labels: std::collections::BTreeMap<String, String>,
        active: bool,
        pending: bool,
        error: Option<String>,
    },
    Status {
        query: String,
        value_from: Option<String>,
        label_map: std::collections::BTreeMap<String, String>,
        tone_map: std::collections::BTreeMap<String, String>,
        label: Option<String>,
        tone: StatusTone,
        error: Option<String>,
    },
    QrCode {
        query: String,
        value_from: Option<String>,
        url: Option<String>,
        modules: Vec<bool>,
        error: Option<String>,
    },
    List {
        query: String,
        searchable: bool,
        filter: String,
        active_query: Option<String>,
        active_value_from: Option<String>,
        active_label: Option<String>,
        active: bool,
        row_label: String,
        row_subtitle: Option<String>,
        actions: Box<ListActions>,
        empty_message: String,
        items: Vec<ListItem>,
        list: ScrollList,
        error: Option<String>,
    },
    Gamepad {
        query: String,
        monitor: GamepadMonitor,
    },
    ObjectArray(ObjectArrayState),
    Unsupported {
        kind: FieldKind,
        reason: String,
    },
}

pub(super) struct Row {
    pub(super) id: String,
    pub(super) section_id: Option<String>,
    pub(super) section_label: Option<String>,
    pub(super) label: String,
    pub(super) description: Option<String>,
    pub(super) placeholder: Option<String>,
    pub(super) variant: Option<String>,
    pub(super) config_key: String,
    pub(super) default: FieldDefault,
    pub(super) stream: Option<String>,
    pub(super) action: Option<String>,
    pub(super) visibility: Option<RowVisibility>,
    pub(super) control: RowControl,
}

#[derive(Debug)]
pub(super) struct RowVisibility {
    show_when: ResolvedShowWhen,
    initial_value: FieldDefault,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RowSection {
    pub(super) label: String,
    pub(super) description: Option<String>,
    pub(super) rows: Vec<usize>,
}

pub(super) fn rows_from_resolved(config: &ResolvedConfig) -> Vec<Row> {
    let field_values = resolved_field_values(config);
    let mut rows = Vec::new();
    for field in &config.fields {
        push_row(&mut rows, None, None, field, &field_values);
    }
    for section in &config.sections {
        push_section_rows(&mut rows, section, &field_values);
    }
    rows
}

pub(super) fn sections_from_resolved(config: &ResolvedConfig, rows: &[Row]) -> Vec<RowSection> {
    let mut sections = Vec::new();
    let root_rows = rows
        .iter()
        .enumerate()
        .filter(|(_, row)| row.section_id.is_none())
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if !root_rows.is_empty() {
        sections.push(RowSection {
            label: "General".into(),
            description: config.description.clone(),
            rows: root_rows,
        });
    }
    sections.extend(config.sections.iter().filter_map(|section| {
        let section_rows = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.section_id.as_deref() == Some(section.id.as_str()))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if section_rows.is_empty() {
            return None;
        }
        Some(RowSection {
            label: section.label.clone(),
            description: section.description.clone(),
            rows: section_rows,
        })
    }));
    sections
}

fn resolved_field_values(
    config: &ResolvedConfig,
) -> std::collections::BTreeMap<String, FieldDefault> {
    config
        .fields
        .iter()
        .chain(config.sections.iter().flat_map(|section| &section.fields))
        .map(|field| (field.id.clone(), field.value.clone()))
        .collect()
}

fn push_section_rows(
    rows: &mut Vec<Row>,
    section: &ResolvedSection,
    field_values: &std::collections::BTreeMap<String, FieldDefault>,
) {
    let mut label = Some(section.label.clone());
    for field in &section.fields {
        let before = rows.len();
        push_row(
            rows,
            Some(section.id.clone()),
            label.clone(),
            field,
            field_values,
        );
        if rows.len() > before {
            label = None;
        }
    }
}

fn push_row(
    rows: &mut Vec<Row>,
    section_id: Option<String>,
    section_label: Option<String>,
    field: &ResolvedField,
    field_values: &std::collections::BTreeMap<String, FieldDefault>,
) {
    let control = control_for(field);
    let visibility = field.show_when.clone().and_then(|show_when| {
        field_values
            .get(&show_when.field)
            .cloned()
            .map(|initial_value| RowVisibility {
                show_when,
                initial_value,
            })
    });
    rows.push(Row {
        id: field.id.clone(),
        section_id,
        section_label,
        label: field.label.clone(),
        description: field.description.clone(),
        placeholder: field.placeholder.clone(),
        variant: field.variant.clone(),
        config_key: field.config_key.clone(),
        default: field.default.clone(),
        stream: field.stream.clone(),
        action: field.action.clone(),
        visibility,
        control,
    });
}

pub(super) fn row_streams(rows: &[Row], index: usize) -> bool {
    rows.get(index)
        .and_then(|row| row.stream.as_deref())
        .is_some_and(|stream| !stream.is_empty())
}

pub(super) fn row_action(rows: &[Row], index: usize) -> Option<String> {
    rows.get(index)
        .and_then(|row| row.action.as_deref())
        .map(str::to_string)
}

pub(super) fn stream_gated(rows: &[Row]) -> bool {
    rows.iter().any(|row| match &row.control {
        RowControl::Status { tone, error, .. } => *tone == StatusTone::Danger || error.is_some(),
        _ => false,
    })
}

pub(super) fn row_is_visible(rows: &[Row], index: usize) -> bool {
    let Some(visibility) = rows.get(index).and_then(|row| row.visibility.as_ref()) else {
        return true;
    };
    let current = rows
        .iter()
        .find(|row| row.id == visibility.show_when.field)
        .and_then(|row| row_value(&row.control))
        .unwrap_or_else(|| visibility.initial_value.clone());
    current == visibility.show_when.equals
}

#[cfg(test)]
pub(super) fn visible_row_indices(rows: &[Row]) -> Vec<usize> {
    (0..rows.len())
        .filter(|index| row_is_visible(rows, *index))
        .collect()
}

pub(super) fn section_label_for(rows: &[Row], index: usize) -> Option<&str> {
    let section_id = rows.get(index)?.section_id.as_deref()?;
    let previous_visible = (0..index)
        .rev()
        .find(|candidate| row_is_visible(rows, *candidate));
    if previous_visible
        .is_some_and(|candidate| rows[candidate].section_id.as_deref() == Some(section_id))
    {
        return None;
    }
    rows.iter()
        .find(|row| row.section_id.as_deref() == Some(section_id))
        .and_then(|row| row.section_label.as_deref())
}

fn control_for(field: &ResolvedField) -> RowControl {
    match field.kind {
        FieldKind::Boolean => match field.value {
            FieldDefault::Boolean(value) => RowControl::Toggle(value),
            _ => unsupported_mismatch(field),
        },
        FieldKind::Select => {
            let current = match &field.value {
                FieldDefault::String(value) => value.clone(),
                _ => return unsupported_mismatch(field),
            };
            let (options, dynamic) = field_options(field, std::slice::from_ref(&current));
            let index = options
                .iter()
                .position(|option| option.value == current)
                .unwrap_or(0);
            RowControl::Select {
                options,
                index,
                dynamic,
            }
        }
        FieldKind::Number => match field.value {
            FieldDefault::Number(value) => RowControl::Number {
                value,
                min: field.number.min,
                max: field.number.max,
                step: field.number.step,
            },
            _ => unsupported_mismatch(field),
        },
        FieldKind::String => match &field.value {
            FieldDefault::String(value) => RowControl::Text(value.clone()),
            _ => unsupported_mismatch(field),
        },
        FieldKind::StringArray => match &field.value {
            FieldDefault::StringArray(values) => {
                if field.options.is_empty() && field.query.is_none() {
                    RowControl::TextList(values.clone())
                } else {
                    let (options, dynamic) = field_options(field, values);
                    RowControl::MultiSelect {
                        selected: options
                            .iter()
                            .map(|option| values.contains(&option.value))
                            .collect(),
                        options,
                        dynamic,
                    }
                }
            }
            _ => unsupported_mismatch(field),
        },
        FieldKind::Color => match &field.value {
            FieldDefault::String(value) => RowControl::Color(value.clone()),
            _ => unsupported_mismatch(field),
        },
        FieldKind::Action => match field.action.as_deref() {
            Some(action) => RowControl::Action {
                action: action.to_string(),
                active_action: field.active_action.clone(),
                active_label: field.active_label.clone(),
                active_query: field.active_query.clone(),
                active_value_from: field.active_value_from.clone(),
                state_labels: field
                    .label_map
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                active: false,
                pending: false,
                error: None,
            },
            None => RowControl::Unsupported {
                kind: FieldKind::Action,
                reason: "action field declares no action".into(),
            },
        },
        FieldKind::Status => match field.query.as_deref() {
            Some(query) => RowControl::Status {
                query: query.to_string(),
                value_from: field.value_from.clone(),
                label_map: field
                    .label_map
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                tone_map: field
                    .tone_map
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                label: None,
                tone: StatusTone::Muted,
                error: None,
            },
            None => RowControl::Unsupported {
                kind: FieldKind::Status,
                reason: "status field declares no query".into(),
            },
        },
        FieldKind::List => match field.query.as_deref() {
            Some(query) => RowControl::List {
                query: query.to_string(),
                searchable: field.search == Some(true),
                filter: String::new(),
                active_query: field.active_query.clone(),
                active_value_from: field.active_value_from.clone(),
                active_label: field.active_label.clone(),
                active: false,
                row_label: field.row_label.clone().unwrap_or_else(|| "{name}".into()),
                row_subtitle: field.row_subtitle.clone(),
                actions: Box::new(ListActions {
                    primary: field.row_action.clone(),
                    additional: field.row_actions.clone(),
                }),
                empty_message: field
                    .empty_message
                    .clone()
                    .unwrap_or_else(|| "No items.".into()),
                items: Vec::new(),
                list: ScrollList::new(LIST_MAX_VISIBLE),
                error: None,
            },
            None => RowControl::Unsupported {
                kind: FieldKind::List,
                reason: "list field declares no query".into(),
            },
        },
        FieldKind::Gamepad => match field.query.as_deref() {
            Some(query) => RowControl::Gamepad {
                query: query.to_string(),
                monitor: GamepadMonitor::default(),
            },
            None => RowControl::Unsupported {
                kind: FieldKind::Gamepad,
                reason: "gamepad field declares no query".into(),
            },
        },
        FieldKind::ObjectArray => match &field.value {
            FieldDefault::ObjectArray(items) => RowControl::ObjectArray(ObjectArrayState::list(
                item_schema(field.item.as_ref(), items),
                items.clone(),
            )),
            _ => unsupported_mismatch(field),
        },
        FieldKind::ObjectMap => match &field.value {
            FieldDefault::ObjectMap(entries) => RowControl::ObjectArray(ObjectArrayState::map(
                field.key_label.clone().unwrap_or_else(|| "Key".into()),
                entry_field_schema(field, entries),
                entries
                    .iter()
                    .map(|(key, fields)| (key.clone(), fields.clone()))
                    .collect(),
            )),
            _ => unsupported_mismatch(field),
        },
        FieldKind::QrCode => RowControl::QrCode {
            query: field.query.clone().unwrap_or_default(),
            value_from: field.value_from.clone(),
            url: None,
            modules: Vec::new(),
            error: None,
        },
    }
}

fn unsupported_mismatch(field: &ResolvedField) -> RowControl {
    RowControl::Unsupported {
        kind: field.kind,
        reason: format!(
            "{} field holds {}",
            field.kind.name(),
            shape_with_article(&field.value)
        ),
    }
}

fn shape_with_article(value: &FieldDefault) -> String {
    match stored_shape(value) {
        shape @ ("object_array" | "object_map") => format!("an {shape}"),
        shape => format!("a {shape}"),
    }
}

fn stored_shape(value: &FieldDefault) -> &'static str {
    match value {
        FieldDefault::Boolean(_) => "boolean",
        FieldDefault::String(_) => "string",
        FieldDefault::Number(_) => "number",
        FieldDefault::StringArray(_) => "string_array",
        FieldDefault::ObjectArray(_) => "object_array",
        FieldDefault::ObjectMap(_) => "object_map",
    }
}

fn entry_field_schema(
    field: &ResolvedField,
    entries: &qol_config::contract::IndexMap<
        String,
        qol_config::contract::IndexMap<String, FieldDefault>,
    >,
) -> Vec<qol_config::object_array::ItemField> {
    if field.entry_fields.is_empty() {
        return item_schema(None, &entries.values().cloned().collect::<Vec<_>>());
    }
    item_schema(
        Some(&qol_config::normalized::ResolvedItemSpec {
            fields: field.entry_fields.clone(),
        }),
        &[],
    )
}

fn field_options(
    field: &ResolvedField,
    current: &[String],
) -> (Vec<SelectOption>, Option<OptionQuery>) {
    let seeded = seeded_options(field);
    let options = merge_options(&seeded, &[], current);
    let dynamic = field.query.as_ref().map(|name| OptionQuery {
        name: name.clone(),
        seeded,
    });
    (options, dynamic)
}

fn seeded_options(field: &ResolvedField) -> Vec<SelectOption> {
    let mut seeded = field
        .options
        .iter()
        .map(|option| {
            let label = field
                .option_labels
                .get(option)
                .cloned()
                .unwrap_or_else(|| option.clone());
            SelectOption::plain(option, label)
        })
        .collect::<Vec<_>>();
    if field.query.is_some() {
        for (option, label) in &field.option_labels {
            if !seeded.iter().any(|candidate| &candidate.value == option) {
                seeded.push(SelectOption::plain(option, label));
            }
        }
    }
    seeded
}

fn merge_options(
    seeded: &[SelectOption],
    dynamic: &[SelectOption],
    current: &[String],
) -> Vec<SelectOption> {
    let mut merged = seeded.to_vec();
    for option in dynamic {
        if let Some(seeded) = merged
            .iter_mut()
            .find(|candidate| candidate.value == option.value)
        {
            seeded.accent = option.accent;
            continue;
        }
        merged.push(option.clone());
    }
    for value in current.iter().rev() {
        if !merged.iter().any(|option| &option.value == value) {
            merged.insert(0, SelectOption::plain(value, value));
        }
    }
    merged
}

pub(super) fn merged_config(base: &serde_json::Value, rows: &[Row]) -> serde_json::Value {
    let mut config = if base.is_object() {
        base.clone()
    } else {
        serde_json::json!({})
    };
    for row in rows {
        match &row.control {
            RowControl::Unsupported { kind, .. } if kind.has_stored_value() => {
                set_config_value(
                    &mut config,
                    &row.config_key,
                    qol_config::field_default_to_json(&row.default),
                );
            }
            RowControl::Unsupported { .. } => {}
            control => {
                if let Some(value) = row_value_json(control) {
                    set_config_value(&mut config, &row.config_key, value);
                }
            }
        }
    }
    config
}

fn row_value_json(control: &RowControl) -> Option<serde_json::Value> {
    match control {
        RowControl::Toggle(value) => Some(serde_json::json!(value)),
        RowControl::Select { options, index, .. } => Some(serde_json::json!(options[*index].value)),
        RowControl::MultiSelect {
            options, selected, ..
        } => {
            let values: Vec<&String> = options
                .iter()
                .zip(selected)
                .filter(|(_, on)| **on)
                .map(|(option, _)| &option.value)
                .collect();
            Some(serde_json::json!(values))
        }
        RowControl::Number { value, .. } => Some(number_json(*value)),
        RowControl::Text(value) => Some(serde_json::json!(value)),
        RowControl::TextList(values) => Some(serde_json::json!(values)),
        RowControl::Color(value) => Some(serde_json::json!(value)),
        RowControl::ObjectArray(state) => Some(qol_config::field_default_to_json(
            &object_array_value(state),
        )),
        RowControl::Action { .. }
        | RowControl::List { .. }
        | RowControl::Status { .. }
        | RowControl::Gamepad { .. }
        | RowControl::QrCode { .. }
        | RowControl::Unsupported { .. } => None,
    }
}

fn object_array_value(state: &ObjectArrayState) -> FieldDefault {
    match state.key_label.is_some() {
        true => FieldDefault::ObjectMap(state.keyed_items()),
        false => FieldDefault::ObjectArray(state.items()),
    }
}

fn row_value(control: &RowControl) -> Option<FieldDefault> {
    match control {
        RowControl::Toggle(value) => Some(FieldDefault::Boolean(*value)),
        RowControl::Select { options, index, .. } => {
            Some(FieldDefault::String(options[*index].value.clone()))
        }
        RowControl::MultiSelect {
            options, selected, ..
        } => {
            let values = options
                .iter()
                .zip(selected)
                .filter(|(_, on)| **on)
                .map(|(option, _)| option.value.clone())
                .collect();
            Some(FieldDefault::StringArray(values))
        }
        RowControl::Number { value, .. } => Some(FieldDefault::Number(*value)),
        RowControl::Text(value) | RowControl::Color(value) => {
            Some(FieldDefault::String(value.clone()))
        }
        RowControl::TextList(values) => Some(FieldDefault::StringArray(values.clone())),
        RowControl::ObjectArray(state) => Some(object_array_value(state)),
        RowControl::Action { .. }
        | RowControl::Status { .. }
        | RowControl::List { .. }
        | RowControl::Gamepad { .. }
        | RowControl::QrCode { .. }
        | RowControl::Unsupported { .. } => None,
    }
}

pub(super) fn runtime_query_names(rows: &[Row]) -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    for row in rows {
        match &row.control {
            RowControl::Select {
                dynamic: Some(dynamic),
                ..
            }
            | RowControl::MultiSelect {
                dynamic: Some(dynamic),
                ..
            } => {
                names.insert(dynamic.name.clone());
            }
            RowControl::Action {
                active_query: Some(query),
                ..
            } => {
                names.insert(query.clone());
            }
            RowControl::Status { query, .. } => {
                names.insert(query.clone());
            }
            RowControl::List {
                query,
                active_query,
                ..
            } => {
                names.insert(query.clone());
                names.extend(active_query.iter().cloned());
            }
            RowControl::Gamepad { query, .. } => {
                names.insert(query.clone());
            }
            RowControl::QrCode { query, .. } if !query.is_empty() => {
                names.insert(query.clone());
            }
            _ => {}
        }
    }
    names.into_iter().collect()
}

pub(super) fn apply_runtime_query(
    rows: &mut [Row],
    query: &str,
    result: Result<serde_json::Value, String>,
) {
    for row in rows {
        match &mut row.control {
            RowControl::Select {
                options,
                index,
                dynamic: Some(dynamic),
            } if dynamic.name == query => {
                if let Ok(value) = &result {
                    let current = options
                        .get(*index)
                        .map(|option| option.value.clone())
                        .unwrap_or_default();
                    let fetched = options_from_value(value);
                    *options =
                        merge_options(&dynamic.seeded, &fetched, std::slice::from_ref(&current));
                    *index = options
                        .iter()
                        .position(|option| option.value == current)
                        .unwrap_or(0);
                }
            }
            RowControl::MultiSelect {
                options,
                selected,
                dynamic: Some(dynamic),
            } if dynamic.name == query => {
                if let Ok(value) = &result {
                    let current = options
                        .iter()
                        .zip(selected.iter())
                        .filter(|(_, selected)| **selected)
                        .map(|(option, _)| option.value.clone())
                        .collect::<Vec<_>>();
                    let fetched = options_from_value(value);
                    *options = merge_options(&dynamic.seeded, &fetched, &current);
                    *selected = options
                        .iter()
                        .map(|option| current.contains(&option.value))
                        .collect();
                }
            }
            RowControl::Action {
                active_query: Some(active_query),
                active_value_from,
                active,
                error,
                ..
            } if active_query == query => match &result {
                Ok(value) => {
                    *active = query_flag(value, active_value_from.as_deref());
                    *error = None;
                }
                Err(message) => *error = Some(message.clone()),
            },
            RowControl::Status {
                query: row_query,
                value_from,
                label_map,
                tone_map,
                label,
                tone,
                error,
            } if row_query == query => match &result {
                Ok(value) => {
                    let raw = query_value(value, value_from.as_deref()).and_then(query_value_text);
                    *label = raw
                        .as_ref()
                        .and_then(|value| label_map.get(value))
                        .cloned()
                        .or(raw.clone());
                    *tone = raw
                        .as_ref()
                        .and_then(|value| tone_map.get(value))
                        .map(|value| status_tone(value))
                        .unwrap_or(StatusTone::Muted);
                    *error = None;
                }
                Err(message) => *error = Some(message.clone()),
            },
            RowControl::List {
                query: row_query,
                active_query,
                active_value_from,
                active,
                row_label,
                row_subtitle,
                items,
                list,
                error,
                ..
            } => {
                if row_query == query {
                    match &result {
                        Ok(value) => {
                            *items = list_items(value, row_label, row_subtitle.as_deref());
                            list.sync(items.len());
                            *error = None;
                        }
                        Err(message) => *error = Some(message.clone()),
                    }
                }
                if active_query.as_deref() == Some(query) {
                    *active = result
                        .as_ref()
                        .is_ok_and(|value| query_flag(value, active_value_from.as_deref()));
                }
            }
            RowControl::Gamepad {
                query: row_query,
                monitor,
            } if row_query == query => monitor.apply_query(result.clone()),
            RowControl::QrCode {
                query: row_query,
                value_from,
                url,
                modules,
                error,
            } if row_query == query => match &result {
                Ok(value) => {
                    let raw = value_from
                        .as_deref()
                        .and_then(|path| query_value(value, Some(path)));
                    let next = raw.and_then(qr_url_text);
                    match next {
                        Some(text) if url.as_ref() != Some(&text) => match qr_modules(&text) {
                            Ok(encoded) => {
                                *url = Some(text);
                                *modules = encoded;
                                *error = None;
                            }
                            Err(message) => *error = Some(message),
                        },
                        Some(_) => {}
                        None => {
                            *url = None;
                            modules.clear();
                            *error = None;
                        }
                    }
                }
                Err(message) => *error = Some(message.clone()),
            },
            _ => {}
        }
    }
}

fn query_flag(value: &serde_json::Value, path: Option<&str>) -> bool {
    query_flag_value(value, path).unwrap_or(false)
}

pub(super) fn query_flag_value(value: &serde_json::Value, path: Option<&str>) -> Option<bool> {
    query_value(value, path).and_then(serde_json::Value::as_bool)
}

fn query_value<'a>(
    value: &'a serde_json::Value,
    path: Option<&str>,
) -> Option<&'a serde_json::Value> {
    let Some(path) = path.filter(|path| !path.is_empty()) else {
        return Some(value);
    };
    path.split('.').try_fold(value, |value, key| value.get(key))
}

fn query_value_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            None
        }
    }
}

fn qr_url_text(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Number(number) => Some(number.to_string()),
        serde_json::Value::Bool(flag) => Some(flag.to_string()),
        serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            None
        }
    }
}

fn qr_modules(url: &str) -> Result<Vec<bool>, String> {
    let code = qrcode::QrCode::new(url).map_err(|error| error.to_string())?;
    let width = code.width();
    let quiet = 4;
    let side = width + 2 * quiet;
    let mut modules = vec![false; side * side];
    for (index, color) in code.to_colors().into_iter().enumerate() {
        let x = index % width;
        let y = index / width;
        modules[(y + quiet) * side + (x + quiet)] = color == qrcode::Color::Dark;
    }
    Ok(modules)
}

fn status_tone(value: &str) -> StatusTone {
    match value {
        "accent" => StatusTone::Accent,
        "success" => StatusTone::Success,
        "danger" | "error" => StatusTone::Danger,
        "warning" => StatusTone::Warning,
        _ => StatusTone::Muted,
    }
}

fn list_items(value: &serde_json::Value, label: &str, subtitle: Option<&str>) -> Vec<ListItem> {
    let rows = value
        .get("items")
        .unwrap_or(value)
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    rows.iter()
        .enumerate()
        .map(|(index, row)| ListItem {
            id: row
                .get("id")
                .or_else(|| row.get("address"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| index.to_string()),
            label: render_template(label, row),
            subtitle: subtitle
                .map(|template| render_template(template, row))
                .filter(|text| !text.is_empty()),
            accent: list_item_tone(row, "accent"),
            badge: row
                .get("badge")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            badge_tone: list_item_tone(row, "badge_tone"),
            data: row.clone(),
            pending: row
                .get("action_pending")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            error: None,
        })
        .collect()
}

fn list_item_tone(row: &serde_json::Value, key: &str) -> Option<StatusTone> {
    row.get(key)
        .and_then(serde_json::Value::as_str)
        .and_then(StatusTone::from_contract)
}

pub(super) fn primary_list_item_action(
    actions: &ListActions,
    item: &ListItem,
) -> Option<ResolvedRowAction> {
    list_item_actions(actions, item).into_iter().next()
}

pub(super) fn list_item_actions(actions: &ListActions, item: &ListItem) -> Vec<ResolvedRowAction> {
    resolve_row_actions(actions.primary.as_ref(), &actions.additional, &item.data)
}

pub(super) fn begin_list_item_action(
    item: &mut ListItem,
    action: ResolvedRowAction,
) -> Option<ResolvedRowAction> {
    if item.pending {
        return None;
    }
    item.pending = true;
    item.error = None;
    Some(action)
}

pub(super) fn filtered_list_items(
    actions: &ListActions,
    items: &[ListItem],
    filter: &str,
) -> Vec<usize> {
    let normalized = filter.trim();
    if normalized.is_empty() {
        return (0..items.len()).collect();
    }
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| list_matches_filter(actions, item, normalized))
        .map(|(index, _)| index)
        .collect()
}

pub(super) fn list_matches_filter(actions: &ListActions, item: &ListItem, filter: &str) -> bool {
    let filter = filter.to_lowercase();
    if item.label.to_lowercase().contains(&filter) {
        return true;
    }
    if item
        .subtitle
        .as_deref()
        .is_some_and(|subtitle| subtitle.to_lowercase().contains(&filter))
    {
        return true;
    }
    list_item_actions(actions, item)
        .iter()
        .any(|action| action.label.to_lowercase().contains(&filter))
}

pub(super) fn apply_list_filter(
    actions: &ListActions,
    items: &[ListItem],
    list: &mut ScrollList,
    filter: &mut String,
    next: String,
) -> usize {
    *filter = next;
    let count = filtered_list_items(actions, items, filter).len();
    list.reset();
    list.sync(count);
    count
}

fn render_template(template: &str, row: &serde_json::Value) -> String {
    let mut rendered = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        rendered.push_str(&rest[..open]);
        let tail = &rest[open + 1..];
        let Some(close) = tail.find('}') else {
            rendered.push_str(&rest[open..]);
            return rendered;
        };
        let key = &tail[..close];
        if !template_key(key) {
            rendered.push('{');
            rest = tail;
            continue;
        }
        if let Some(value) = row.get(key).filter(|value| !value.is_null()) {
            let text = value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string());
            rendered.push_str(&text);
        }
        rest = &tail[close + 1..];
    }
    rendered.push_str(rest);
    rendered
}

fn template_key(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn options_from_value(value: &serde_json::Value) -> Vec<SelectOption> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            if let Some(value) = item.as_str() {
                return Some(SelectOption::plain(value, value));
            }
            let value = item.get("value")?.as_str()?.to_string();
            let label = item
                .get("label")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&value)
                .to_string();
            Some(SelectOption {
                value,
                label,
                accent: option_accent(item),
            })
        })
        .collect()
}

fn option_accent(item: &serde_json::Value) -> Option<u32> {
    let color = item.get("accent")?;
    let red = u8::try_from(color.get("red")?.as_u64()?).ok()?;
    let green = u8::try_from(color.get("green")?.as_u64()?).ok()?;
    let blue = u8::try_from(color.get("blue")?.as_u64()?).ok()?;
    Some(u32::from(red) << 16 | u32::from(green) << 8 | u32::from(blue))
}

fn number_json(value: f64) -> serde_json::Value {
    let whole = value.fract() == 0.0 && value >= i64::MIN as f64 && value <= i64::MAX as f64;
    if whole {
        serde_json::json!(value as i64)
    } else {
        serde_json::json!(value)
    }
}

fn set_config_value(root: &mut serde_json::Value, dotted_key: &str, value: serde_json::Value) {
    if !root.is_object() {
        *root = serde_json::json!({});
    }
    let mut cursor = root;
    let mut parts = dotted_key.split('.').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            cursor[part] = value;
            return;
        }
        if !cursor[part].is_object() {
            cursor[part] = serde_json::json!({});
        }
        cursor = &mut cursor[part];
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_list_filter, apply_runtime_query, begin_list_item_action, filtered_list_items,
        list_item_actions, list_items, merged_config, option_accent, primary_list_item_action,
        render_template, row_action, row_is_visible, row_streams, row_value_json,
        rows_from_resolved, runtime_query_names, section_label_for, sections_from_resolved,
        set_config_value, stream_gated, visible_row_indices, FieldDefault, ListActions, ListItem,
        ResolvedConfig, Row, RowControl, SelectOption,
    };
    use crate::status_indicator::StatusTone;
    use qol_config::object_array::ItemFieldKind;

    const SPEC: &str = r#"
schema_version = 1

[section.capture]
label = "Capture"

[field.pin_border]
type = "boolean"
config_key = "capture.pin_border"
label = "Pinned Preview Border"
description = "Keep previews distinct from the desktop."
section = "capture"
default = true

[field.saved_feedback]
type = "select"
config_key = "capture.saved_feedback"
label = "Saved Feedback"
section = "capture"
default = "notification"
options = ["notification", "toast"]

[field.crf]
type = "number"
config_key = "video.crf"
label = "CRF"
section = "capture"
default = 18
min = 0
max = 51
step = 1
variant = "slider"

[field.mic]
type = "string"
config_key = "audio.mic_device"
label = "Mic Device"
placeholder = "System default"
section = "capture"
default = "default"

[field.inputs]
type = "string_array"
config_key = "audio.inputs"
label = "Audio Inputs"
section = "capture"
default = ["mic"]
options = ["mic", "system"]

[field.inputs.option_labels]
mic = "Microphone"
system = "System Audio"

[field.tags]
type = "string_array"
config_key = "capture.tags"
label = "Tags"
section = "capture"
default = ["foo"]

[field.card_color]
type = "color"
config_key = "display.card_color"
label = "Card Color"
section = "capture"
default = "202322"
"#;

    fn resolved(overrides: serde_json::Value) -> ResolvedConfig {
        let spec = qol_config::contract::parse_spec_str(SPEC).unwrap();
        qol_config::normalized::resolve_config(&spec, &overrides).unwrap()
    }

    fn option(value: &str, label: &str) -> SelectOption {
        SelectOption::plain(value, label)
    }

    #[test]
    fn rows_map_every_supported_kind_with_override_values() {
        let rows = rows_from_resolved(&resolved(serde_json::json!({
            "capture": { "pin_border": false, "saved_feedback": "toast" }
        })));
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[0].section_label.as_deref(), Some("Capture"));
        assert!(rows[1..].iter().all(|r| r.section_label.is_none()));
        assert_eq!(
            rows[0].description.as_deref(),
            Some("Keep previews distinct from the desktop.")
        );
        assert_eq!(rows[2].variant.as_deref(), Some("slider"));
        assert_eq!(rows[3].placeholder.as_deref(), Some("System default"));
        assert!(matches!(rows[0].control, RowControl::Toggle(false)));
        match &rows[1].control {
            RowControl::Select { index, options, .. } => {
                assert_eq!(options[*index].value, "toast");
            }
            other => panic!("expected select, got {other:?}"),
        }
        match &rows[2].control {
            RowControl::Number { value, step, .. } => {
                assert_eq!((*value, *step), (18.0, Some(1.0)));
            }
            other => panic!("expected number, got {other:?}"),
        }
        assert!(matches!(&rows[3].control, RowControl::Text(v) if v == "default"));
        match &rows[4].control {
            RowControl::MultiSelect {
                options, selected, ..
            } => {
                assert_eq!(
                    options,
                    &[
                        option("mic", "Microphone"),
                        option("system", "System Audio")
                    ]
                );
                assert_eq!(selected, &[true, false]);
            }
            other => panic!("expected multi select, got {other:?}"),
        }
        assert!(
            matches!(&rows[5].control, RowControl::TextList(v) if v == &vec!["foo".to_string()])
        );
        assert!(matches!(&rows[6].control, RowControl::Color(v) if v == "202322"));
    }

    #[test]
    fn gamepad_fields_share_runtime_query_and_native_monitor_state() {
        let spec = qol_config::contract::parse_spec_str(
            r#"
schema_version = 1

[field.input]
type = "gamepad"
label = "Controller Input"
query = "controller_input"
"#,
        )
        .unwrap();
        let resolved =
            qol_config::normalized::resolve_config(&spec, &serde_json::json!({})).unwrap();
        let mut rows = rows_from_resolved(&resolved);

        assert_eq!(runtime_query_names(&rows), ["controller_input"]);
        apply_runtime_query(
            &mut rows,
            "controller_input",
            Ok(serde_json::json!({
                "available": true,
                "items": [{
                    "name": "foo pad",
                    "state": {"mapping": "standard", "buttons": [], "axes": []},
                }],
            })),
        );

        let RowControl::Gamepad { monitor, .. } = &rows[0].control else {
            panic!("expected gamepad monitor");
        };
        assert_eq!(
            monitor
                .selected()
                .map(|controller| controller.name.as_str()),
            Some("foo pad")
        );
    }

    #[test]
    fn object_array_fields_become_editable_rows_that_round_trip_their_items() {
        let spec = qol_config::contract::parse_spec_str(
            r#"
schema_version = 1

[field.key_rules]
type = "object_array"
label = "Key Rules"
default = [
  { from_mods = ["ctrl"], to_mods = ["cmd"], keys = ["c", "v"] },
]

[field.key_rules.item.fields]
from_mods = "string_array"
to_mods = "string_array"
keys = "string_array"
global = "boolean"
"#,
        )
        .unwrap();
        let resolved =
            qol_config::normalized::resolve_config(&spec, &serde_json::json!({})).unwrap();
        let rows = rows_from_resolved(&resolved);

        let RowControl::ObjectArray(state) = &rows[0].control else {
            panic!("expected an object array row, got {:?}", rows[0].control);
        };
        assert_eq!(
            state.schema,
            vec![
                ("from_mods".to_string(), ItemFieldKind::Mods),
                ("global".to_string(), ItemFieldKind::Boolean),
                ("keys".to_string(), ItemFieldKind::StringArray),
                ("to_mods".to_string(), ItemFieldKind::Mods),
            ]
        );
        assert_eq!(state.entries.len(), 1);
        assert_eq!(
            merged_config(&serde_json::json!({}), &rows),
            serde_json::json!({
                "key_rules": [
                    { "from_mods": ["ctrl"], "to_mods": ["cmd"], "keys": ["c", "v"] },
                ]
            })
        );
    }

    #[test]
    fn an_object_array_defaulting_to_empty_still_gets_a_row() {
        let spec = qol_config::contract::parse_spec_str(
            r#"
schema_version = 1

[section.windows]
label = "Windows"

[field.switchable_panels]
type = "object_array"
label = "Switchable Panels"
section = "windows"
default = []

[field.switchable_panels.item.fields]
app = "string"
switchable = "boolean"
"#,
        )
        .unwrap();
        let resolved =
            qol_config::normalized::resolve_config(&spec, &serde_json::json!({})).unwrap();
        let rows = rows_from_resolved(&resolved);

        assert_eq!(rows.len(), 1, "an empty rule list still needs its editor");
        let RowControl::ObjectArray(state) = &rows[0].control else {
            panic!("expected an object array row, got {:?}", rows[0].control);
        };
        assert!(state.entries.is_empty());
        assert_eq!(
            sections_from_resolved(&resolved, &rows)
                .iter()
                .map(|section| section.label.clone())
                .collect::<Vec<_>>(),
            vec!["Windows".to_string()],
            "a section holding only this field must not disappear"
        );
    }

    #[test]
    fn action_and_list_fields_share_the_contract_order_and_sections() {
        const MIXED_SPEC: &str = r#"
schema_version = 1

[section.devices]
label = "Devices"

[field.search]
type = "action"
label = "Start search"
section = "devices"
action = "start_search"

[field.devices]
type = "list"
label = "Bluetooth devices"
section = "devices"
query = "devices"
row_label = "{name}"

[section.reconnection]
label = "Reconnection"

[field.auto_reconnect]
type = "boolean"
config_key = "auto_reconnect"
label = "Reconnect automatically"
section = "reconnection"
default = true

[field.retry_initial_seconds]
type = "number"
config_key = "retry_initial_seconds"
label = "Initial retry delay"
section = "reconnection"
default = 1
min = 1
max = 60
step = 1
"#;
        let spec = qol_config::contract::parse_spec_str(MIXED_SPEC).unwrap();
        let resolved =
            qol_config::normalized::resolve_config(&spec, &serde_json::json!({})).unwrap();
        let rows = rows_from_resolved(&resolved);

        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].section_label.as_deref(), Some("Devices"));
        assert!(matches!(rows[0].control, RowControl::Action { .. }));
        assert_eq!(rows[1].section_label, None);
        assert!(matches!(rows[1].control, RowControl::List { .. }));
        assert_eq!(rows[2].section_label.as_deref(), Some("Reconnection"));
        assert_eq!(rows[2].config_key, "auto_reconnect");
        assert!(matches!(rows[2].control, RowControl::Toggle(true)));
        assert_eq!(rows[3].section_label, None);
        assert_eq!(rows[3].config_key, "retry_initial_seconds");
        assert!(matches!(
            rows[3].control,
            RowControl::Number { value: 1.0, .. }
        ));
    }

    #[test]
    fn section_navigation_preserves_contract_groups_and_descriptions() {
        const SECTION_SPEC: &str = r#"
schema_version = 1
description = "Root settings"

[field.root]
type = "boolean"
default = true

[section.cursor]
label = "Cursor"
description = "Cursor behavior"

[field.cursor_speed]
type = "number"
section = "cursor"
default = 4

[section.theme]
label = "Theme"
description = "Desktop appearance"

[field.switch]
type = "action"
section = "theme"
action = "switch"
"#;
        let spec = qol_config::contract::parse_spec_str(SECTION_SPEC).unwrap();
        let resolved =
            qol_config::normalized::resolve_config(&spec, &serde_json::json!({})).unwrap();
        let rows = rows_from_resolved(&resolved);
        let sections = sections_from_resolved(&resolved, &rows);

        assert_eq!(sections.len(), 3);
        assert_eq!(sections[0].label, "General");
        assert_eq!(sections[0].description.as_deref(), Some("Root settings"));
        assert_eq!(sections[0].rows, [0]);
        assert_eq!(sections[1].label, "Cursor");
        assert_eq!(sections[1].description.as_deref(), Some("Cursor behavior"));
        assert_eq!(sections[1].rows, [1]);
        assert_eq!(sections[2].label, "Theme");
        assert_eq!(
            sections[2].description.as_deref(),
            Some("Desktop appearance")
        );
        assert_eq!(sections[2].rows, [2]);
    }

    #[test]
    fn conditional_rows_follow_current_controller_values() {
        const CONDITIONAL_SPEC: &str = r#"
schema_version = 1

[section.appearance]
label = "Appearance"

[field.detail]
type = "number"
section = "appearance"
default = 4

[field.detail.show_when]
field = "enabled"
equals = true

[field.enabled]
type = "boolean"
section = "appearance"
default = false

[field.mode]
type = "select"
section = "appearance"
default = "compact"
options = ["compact", "wide"]

[field.width]
type = "number"
section = "appearance"
default = 800

[field.width.show_when]
field = "mode"
equals = "wide"
"#;
        let spec = qol_config::contract::parse_spec_str(CONDITIONAL_SPEC).unwrap();
        let resolved =
            qol_config::normalized::resolve_config(&spec, &serde_json::json!({})).unwrap();
        let mut rows = rows_from_resolved(&resolved);

        assert_eq!(visible_row_indices(&rows), [1, 2]);
        assert!(!row_is_visible(&rows, 0));
        assert_eq!(section_label_for(&rows, 1), Some("Appearance"));
        assert_eq!(section_label_for(&rows, 2), None);

        let RowControl::Toggle(enabled) = &mut rows[1].control else {
            panic!("expected enabled toggle");
        };
        *enabled = true;
        assert_eq!(visible_row_indices(&rows), [0, 1, 2]);
        assert_eq!(section_label_for(&rows, 0), Some("Appearance"));

        let RowControl::Select { index, .. } = &mut rows[2].control else {
            panic!("expected mode select");
        };
        *index = 1;
        assert_eq!(visible_row_indices(&rows), [0, 1, 2, 3]);
    }

    #[test]
    fn query_select_merges_labeled_dynamic_and_current_options() {
        const QUERY_SPEC: &str = r#"
schema_version = 1

[field.mic]
type = "select"
config_key = "audio.mic_device"
label = "Mic Device"
default = "default"
query = "audio_sources"

[field.mic.option_labels]
default = "System Default"
"#;
        let spec = qol_config::contract::parse_spec_str(QUERY_SPEC).unwrap();
        let cases = [
            (
                serde_json::json!({}),
                vec!["default", "alsa_input.foo"],
                vec!["System Default", "Built-in Mic"],
                0usize,
            ),
            (
                serde_json::json!({ "audio": { "mic_device": "gone_device" } }),
                vec!["gone_device", "default", "alsa_input.foo"],
                vec!["gone_device", "System Default", "Built-in Mic"],
                0usize,
            ),
        ];
        for (overrides, expected_options, expected_labels, expected_index) in cases {
            let resolved = qol_config::normalized::resolve_config(&spec, &overrides).unwrap();
            let mut rows = rows_from_resolved(&resolved);
            assert_eq!(runtime_query_names(&rows), ["audio_sources"]);
            apply_runtime_query(
                &mut rows,
                "audio_sources",
                Ok(serde_json::json!([
                    {
                        "value": "alsa_input.foo",
                        "label": "Built-in Mic",
                        "accent": { "red": 130, "green": 170, "blue": 255 }
                    }
                ])),
            );
            match &rows[0].control {
                RowControl::Select { options, index, .. } => {
                    assert_eq!(
                        options
                            .iter()
                            .map(|option| option.value.as_str())
                            .collect::<Vec<_>>(),
                        expected_options,
                        "overrides: {overrides}"
                    );
                    assert_eq!(
                        options
                            .iter()
                            .map(|option| option.label.as_str())
                            .collect::<Vec<_>>(),
                        expected_labels,
                        "overrides: {overrides}"
                    );
                    let dynamic = options
                        .iter()
                        .find(|option| option.value == "alsa_input.foo")
                        .unwrap();
                    assert_eq!(dynamic.accent, Some(0x82aaff));
                    assert_eq!(*index, expected_index, "overrides: {overrides}");
                }
                other => panic!("expected select, got {other:?}"),
            }
        }
    }

    #[test]
    fn dynamic_option_accents_accept_only_complete_rgb_bytes() {
        let cases = [
            (
                serde_json::json!({"accent": {"red": 130, "green": 170, "blue": 255}}),
                Some(0x82aaff),
            ),
            (
                serde_json::json!({"accent": {"red": 256, "green": 170, "blue": 255}}),
                None,
            ),
            (
                serde_json::json!({"accent": {"red": 130, "green": 170}}),
                None,
            ),
            (serde_json::json!({"accent": "accent"}), None),
            (serde_json::json!({}), None),
        ];
        for (input, expected) in cases {
            assert_eq!(option_accent(&input), expected, "input: {input}");
        }
    }

    #[test]
    fn query_options_are_deferred_from_initial_row_creation() {
        const QUERY_SPEC: &str = r#"
schema_version = 1

[field.mic]
type = "select"
config_key = "audio.mic_device"
label = "Mic Device"
default = "default"
query = "audio_sources"
"#;
        let spec = qol_config::contract::parse_spec_str(QUERY_SPEC).unwrap();
        let resolved =
            qol_config::normalized::resolve_config(&spec, &serde_json::json!({})).unwrap();

        let rows = rows_from_resolved(&resolved);

        match &rows[0].control {
            RowControl::Select { options, .. } => {
                assert_eq!(options, &[option("default", "default")])
            }
            other => panic!("expected select, got {other:?}"),
        }
        assert_eq!(runtime_query_names(&rows), ["audio_sources"]);
    }

    #[test]
    fn query_multi_select_merges_dynamic_and_stale_selected_options() {
        const QUERY_SPEC: &str = r#"
schema_version = 1

[field.devices]
type = "string_array"
config_key = "managed_devices"
label = "Managed devices"
default = []
query = "managed_device_options"
"#;
        let spec = qol_config::contract::parse_spec_str(QUERY_SPEC).unwrap();
        let resolved = qol_config::normalized::resolve_config(
            &spec,
            &serde_json::json!({ "managed_devices": ["AA:00", "AA:02"] }),
        )
        .unwrap();
        let mut rows = rows_from_resolved(&resolved);
        assert_eq!(runtime_query_names(&rows), ["managed_device_options"]);
        apply_runtime_query(
            &mut rows,
            "managed_device_options",
            Ok(serde_json::json!([
                { "value": "AA:01", "label": "Headphones · AA:01" },
                { "value": "AA:02", "label": "Keyboard · AA:02" }
            ])),
        );

        match &rows[0].control {
            RowControl::MultiSelect {
                options, selected, ..
            } => {
                assert_eq!(
                    options,
                    &[
                        option("AA:00", "AA:00"),
                        option("AA:01", "Headphones · AA:01"),
                        option("AA:02", "Keyboard · AA:02")
                    ]
                );
                assert_eq!(selected, &[true, false, true]);
            }
            other => panic!("expected multi select, got {other:?}"),
        }
    }

    #[test]
    fn merged_config_preserves_fields_without_rows() {
        let base = serde_json::json!({
            "action_mode": "hold_to_switch",
            "display": { "card_background_color": "aabbcc", "max_columns": 6 }
        });
        let rows = vec![
            Row {
                id: "action_mode".into(),
                section_id: None,
                section_label: None,
                label: "Action Mode".into(),
                description: None,
                placeholder: None,
                variant: None,
                config_key: "action_mode".into(),
                default: FieldDefault::Boolean(false),
                stream: None,
                action: None,
                visibility: None,
                control: RowControl::Select {
                    options: vec![option("hold_to_switch", "Hold"), option("sticky", "Sticky")],
                    index: 1,
                    dynamic: None,
                },
            },
            Row {
                id: "max_columns".into(),
                section_id: None,
                section_label: None,
                label: "Max Columns".into(),
                description: None,
                placeholder: None,
                variant: None,
                config_key: "display.max_columns".into(),
                default: FieldDefault::Boolean(false),
                stream: None,
                action: None,
                visibility: None,
                control: RowControl::Number {
                    value: 4.0,
                    min: None,
                    max: None,
                    step: Some(1.0),
                },
            },
        ];
        assert_eq!(
            merged_config(&base, &rows),
            serde_json::json!({
                "action_mode": "sticky",
                "display": { "card_background_color": "aabbcc", "max_columns": 4 }
            })
        );
    }

    #[test]
    fn number_value_json_emits_integers_for_whole_values() {
        let cases = [
            (6.0, serde_json::json!(6)),
            (0.0, serde_json::json!(0)),
            (-4.0, serde_json::json!(-4)),
            (1.5, serde_json::json!(1.5)),
        ];
        for (value, expected) in cases {
            let control = RowControl::Number {
                value,
                min: None,
                max: None,
                step: Some(1.0),
            };
            assert_eq!(row_value_json(&control), Some(expected), "value: {value}");
        }
    }

    #[test]
    fn color_value_json_keeps_the_stored_string() {
        let control = RowControl::Color("#202322".into());
        assert_eq!(row_value_json(&control), Some(serde_json::json!("#202322")));
    }

    #[test]
    fn multi_select_value_json_preserves_option_order() {
        let control = RowControl::MultiSelect {
            options: vec![
                option("mic", "Microphone"),
                option("system", "System Audio"),
            ],
            selected: vec![false, true],
            dynamic: None,
        };
        assert_eq!(
            row_value_json(&control),
            Some(serde_json::json!(["system"]))
        );
    }

    #[test]
    fn list_items_preserve_supported_status_metadata() {
        let items = list_items(
            &serde_json::json!({
                "items": [
                    { "name": "a", "accent": "success", "badge": "Connected", "badge_tone": "success" },
                    { "name": "b", "accent": "accent", "badge": "Paired" },
                    { "name": "c", "accent": "unknown", "badge": "", "badge_tone": 42 }
                ]
            }),
            "{name}",
            None,
        );
        let cases = [
            (
                Some(StatusTone::Success),
                Some("Connected"),
                Some(StatusTone::Success),
                StatusTone::Success,
            ),
            (
                Some(StatusTone::Accent),
                Some("Paired"),
                None,
                StatusTone::Accent,
            ),
            (None, None, None, StatusTone::Muted),
        ];
        for (item, (accent, badge, badge_tone, effective_tone)) in items.iter().zip(cases) {
            assert_eq!(
                (
                    item.accent,
                    item.badge.as_deref(),
                    item.badge_tone,
                    item.effective_badge_tone(),
                ),
                (accent, badge, badge_tone, effective_tone),
                "item: {}",
                item.label
            );
        }
    }

    #[test]
    fn runtime_queries_update_action_state_and_list_rows() {
        const RUNTIME_SPEC: &str = r#"
schema_version = 1

[field.search]
type = "action"
label = "Start search"
action = "start_search"
active_label = "Stop search"
active_action = "stop_search"
active_query = "search_status"
active_value_from = "searching"

[field.devices]
type = "list"
label = "Bluetooth devices"
query = "devices"
active_query = "search_status"
active_value_from = "searching"
active_label = "LIVE"
row_label = "{name}"
row_subtitle = "{detail}"
empty_message = "No devices."

[[field.devices.row_actions]]
action = "connect_device"
label = "Connect"
when = "can_connect"
input = { address = "{address}" }

[[field.devices.row_actions]]
action = "disconnect_device"
label = "Disconnect"
when = "can_disconnect"
input = { address = "{address}" }

[[field.devices.row_actions]]
action = "remove_device"
label = "Remove"
when = "can_remove"
input = { address = "{address}" }
"#;
        let spec = qol_config::contract::parse_spec_str(RUNTIME_SPEC).unwrap();
        let resolved =
            qol_config::normalized::resolve_config(&spec, &serde_json::json!({})).unwrap();
        let mut rows = rows_from_resolved(&resolved);
        assert_eq!(runtime_query_names(&rows), ["devices", "search_status"]);

        apply_runtime_query(
            &mut rows,
            "search_status",
            Ok(serde_json::json!({ "searching": true })),
        );
        apply_runtime_query(
            &mut rows,
            "devices",
            Ok(serde_json::json!({
                "items": [
                    {
                        "address": "AA:00",
                        "name": "Keyboard",
                        "detail": "Paired · Connected",
                        "can_connect": false,
                        "can_disconnect": true,
                        "can_remove": true
                    },
                    {
                        "address": "AA:01",
                        "name": "Headphones",
                        "detail": "Discovered",
                        "can_connect": true,
                        "can_disconnect": false,
                        "can_remove": false
                    }
                ]
            })),
        );

        assert!(matches!(
            rows[0].control,
            RowControl::Action { active: true, .. }
        ));
        match &rows[1].control {
            RowControl::List {
                actions,
                items,
                active,
                ..
            } => {
                assert_eq!(items.len(), 2);
                assert!(*active);
                assert_eq!(items[0].label, "Keyboard");
                assert_eq!(items[0].subtitle.as_deref(), Some("Paired · Connected"));
                assert_eq!(items[1].label, "Headphones");
                let connected =
                    primary_list_item_action(actions, &items[0]).expect("connected device action");
                assert_eq!(connected.action, "disconnect_device");
                assert_eq!(connected.label, "Disconnect");
                assert_eq!(connected.input, serde_json::json!({ "address": "AA:00" }));
                assert_eq!(
                    list_item_actions(actions, &items[0])
                        .into_iter()
                        .map(|action| action.action)
                        .collect::<Vec<_>>(),
                    ["disconnect_device", "remove_device"]
                );
                let disconnected = primary_list_item_action(actions, &items[1])
                    .expect("disconnected device action");
                assert_eq!(disconnected.action, "connect_device");
                assert_eq!(disconnected.label, "Connect");
                assert_eq!(
                    disconnected.input,
                    serde_json::json!({ "address": "AA:01" })
                );
                let mut item = items[1].clone();
                item.error = Some("previous failure".into());
                assert_eq!(
                    begin_list_item_action(&mut item, disconnected.clone()),
                    Some(disconnected.clone())
                );
                assert!(item.pending);
                assert_eq!(item.error, None);
                assert_eq!(begin_list_item_action(&mut item, disconnected), None);
            }
            other => panic!("expected list, got {other:?}"),
        }
    }

    #[test]
    fn runtime_status_rows_map_labels_and_tones() {
        const STATUS_SPEC: &str = r#"
schema_version = 1

[field.theme]
type = "status"
query = "theme_status"
value_from = "scheme"
label_map = { light = "Light", dark = "Dark" }
tone_map = { light = "muted", dark = "accent" }
"#;
        let spec = qol_config::contract::parse_spec_str(STATUS_SPEC).unwrap();
        let resolved =
            qol_config::normalized::resolve_config(&spec, &serde_json::json!({})).unwrap();
        let mut rows = rows_from_resolved(&resolved);
        assert_eq!(runtime_query_names(&rows), ["theme_status"]);

        apply_runtime_query(
            &mut rows,
            "theme_status",
            Ok(serde_json::json!({ "scheme": "dark" })),
        );
        assert!(matches!(
            &rows[0].control,
            RowControl::Status {
                label: Some(label),
                tone: StatusTone::Accent,
                error: None,
                ..
            } if label == "Dark"
        ));

        apply_runtime_query(&mut rows, "theme_status", Err("unavailable".into()));
        assert!(matches!(
            &rows[0].control,
            RowControl::Status {
                error: Some(error),
                ..
            } if error == "unavailable"
        ));
    }

    #[test]
    fn runtime_rows_are_not_written_into_plugin_config() {
        let rows = vec![Row {
            id: "search".into(),
            section_id: None,
            section_label: None,
            label: "Start search".into(),
            description: None,
            placeholder: None,
            variant: None,
            config_key: "search".into(),
            default: FieldDefault::Boolean(false),
            stream: None,
            action: None,
            visibility: None,
            control: RowControl::Action {
                action: "start_search".into(),
                active_action: None,
                active_label: None,
                active_query: None,
                active_value_from: None,
                state_labels: std::collections::BTreeMap::new(),
                active: false,
                pending: false,
                error: None,
            },
        }];
        let base = serde_json::json!({ "auto_reconnect": true });
        assert_eq!(merged_config(&base, &rows), base);
    }

    #[test]
    fn set_config_value_creates_nested_paths_and_overwrites() {
        let mut root = serde_json::json!({ "capture": { "pin_border": true } });
        set_config_value(
            &mut root,
            "capture.saved_feedback",
            serde_json::json!("toast"),
        );
        set_config_value(
            &mut root,
            "audio.inputs",
            serde_json::json!(["mic", "system"]),
        );
        assert_eq!(
            root,
            serde_json::json!({
                "capture": { "pin_border": true, "saved_feedback": "toast" },
                "audio": { "inputs": ["mic", "system"] }
            })
        );
    }

    #[test]
    fn unsupported_rows_save_their_default_so_other_edits_still_persist() {
        let spec = qol_config::contract::parse_spec_str(
            r#"
schema_version = 1

[field.enabled]
type = "boolean"
config_key = "enabled"
default = false

[field.mode]
type = "boolean"
config_key = "mode"
default = true
"#,
        )
        .unwrap();
        let stored = serde_json::json!({ "enabled": false, "mode": "yes" });
        let resolved = qol_config::normalized::resolve_config(&spec, &stored).unwrap();
        let mut rows = rows_from_resolved(&resolved);
        assert!(
            matches!(rows[1].control, RowControl::Unsupported { .. }),
            "the stored \"yes\" keeps the mode row unsupported for this session"
        );
        let RowControl::Toggle(enabled) = &mut rows[0].control else {
            panic!("expected enabled toggle");
        };
        *enabled = true;
        assert_eq!(
            merged_config(&stored, &rows),
            serde_json::json!({ "enabled": true, "mode": true }),
            "the unsupported row falls back to its contract default so the save is accepted"
        );
    }

    #[test]
    fn every_contract_field_produces_a_row() {
        let plugins_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
        let mut contracts = Vec::new();
        for entry in std::fs::read_dir(&plugins_root).expect("plugins directory") {
            let plugin_dir = entry.expect("plugin entry").path();
            let contract_path = plugin_dir.join("qol-config.toml");
            if !contract_path.is_file() {
                continue;
            }
            let plugin = plugin_dir
                .file_name()
                .and_then(|name| name.to_str())
                .expect("plugin name")
                .to_string();
            let spec = qol_config::contract::parse_spec(&contract_path)
                .unwrap_or_else(|error| panic!("{plugin}: {error:?}"));
            contracts.push((plugin, spec));
        }
        assert_eq!(contracts.len(), 14, "expected 14 plugin contracts");
        let mut unsupported = Vec::new();
        for (plugin, spec) in &contracts {
            let resolved = qol_config::normalized::resolve_config(spec, &serde_json::json!({}))
                .unwrap_or_else(|errors| panic!("{plugin}: {errors:?}"));
            let field_count = resolved.fields.len()
                + resolved
                    .sections
                    .iter()
                    .map(|section| section.fields.len())
                    .sum::<usize>();
            let rows = rows_from_resolved(&resolved);
            for field in resolved
                .fields
                .iter()
                .chain(resolved.sections.iter().flat_map(|section| &section.fields))
            {
                assert!(
                    rows.iter().any(|row| row.id == field.id),
                    "{plugin}: field {} ({}) produced no row",
                    field.id,
                    field.kind.name()
                );
            }
            assert_eq!(
                rows.len(),
                field_count,
                "{plugin}: {field_count} contract fields must produce exactly {field_count} rows"
            );
            for row in &rows {
                if let RowControl::Unsupported { reason, .. } = &row.control {
                    unsupported.push((plugin.clone(), row.id.clone(), reason.clone()));
                }
            }
        }
        assert!(
            unsupported.is_empty(),
            "every contract field must render, found unsupported rows: {unsupported:?}"
        );
    }

    #[test]
    fn qr_code_rows_encode_the_query_url_and_clear_with_the_payload() {
        let spec = qol_config::contract::parse_spec_str(
            r#"
schema_version = 1

[field.download_qr]
type = "qr_code"
label = "Scan to download"
query = "connection_info"
value_from = "app_download_url"
"#,
        )
        .unwrap();
        let resolved =
            qol_config::normalized::resolve_config(&spec, &serde_json::json!({})).unwrap();
        let mut rows = rows_from_resolved(&resolved);
        assert_eq!(runtime_query_names(&rows), ["connection_info"]);
        let RowControl::QrCode { url, modules, .. } = &rows[0].control else {
            panic!("expected a qr code row, got {:?}", rows[0].control);
        };
        assert_eq!(url, &None);
        assert!(modules.is_empty());

        apply_runtime_query(
            &mut rows,
            "connection_info",
            Ok(serde_json::json!({ "app_download_url": "http://192.168.1.5:45455/app" })),
        );
        let RowControl::QrCode {
            url,
            modules,
            error,
            ..
        } = &rows[0].control
        else {
            panic!("expected a qr code row");
        };
        assert_eq!(url.as_deref(), Some("http://192.168.1.5:45455/app"));
        assert!(!modules.is_empty());
        assert_eq!(error, &None);
        let side = (modules.len() as f64).sqrt() as usize;
        assert_eq!(
            side * side,
            modules.len(),
            "quiet zone keeps the matrix square"
        );

        apply_runtime_query(
            &mut rows,
            "connection_info",
            Ok(serde_json::json!({ "hostname": "host" })),
        );
        let RowControl::QrCode { url, modules, .. } = &rows[0].control else {
            panic!("expected a qr code row");
        };
        assert_eq!(url, &None, "a payload without the path clears the code");
        assert!(modules.is_empty());

        apply_runtime_query(&mut rows, "connection_info", Err("unavailable".into()));
        let RowControl::QrCode { error, .. } = &rows[0].control else {
            panic!("expected a qr code row");
        };
        assert_eq!(error.as_deref(), Some("unavailable"));

        assert_eq!(
            merged_config(&serde_json::json!({ "other": 1 }), &rows),
            serde_json::json!({ "other": 1 }),
            "qr code rows are display only"
        );
    }

    #[derive(Debug)]
    enum ExpectedControl {
        Toggle(bool),
        Number(f64),
        Text(&'static str),
        SelectValue(&'static str),
        TextList(Vec<&'static str>),
        Color(&'static str),
        Unsupported(&'static str),
    }

    fn coercion_spec(kind: &str) -> String {
        let body = match kind {
            "boolean" => "default = true",
            "number" => "default = 0",
            "string" => "default = \"d\"",
            "select" => "default = \"a\"\noptions = [\"a\", \"b\"]",
            "color" => "default = \"202322\"",
            "string_array" => "default = []",
            "object_array" => "default = []\n\n[field.value.item.fields]\napp = \"string\"",
            "object_map" => "default = { entry = { value = 1 } }",
            _ => panic!("unknown kind {kind}"),
        };
        format!("schema_version = 1\n\n[field.value]\ntype = \"{kind}\"\n{body}\n")
    }

    fn assert_expected_control(rows: &[Row], label: &str, expected: &ExpectedControl) {
        let got = &rows[0].control;
        match (got, expected) {
            (RowControl::Toggle(value), ExpectedControl::Toggle(want)) => {
                assert_eq!(value, want, "{label}");
            }
            (RowControl::Number { value, .. }, ExpectedControl::Number(want)) => {
                assert_eq!(value, want, "{label}");
            }
            (RowControl::Text(value), ExpectedControl::Text(want)) => {
                assert_eq!(value, want, "{label}");
            }
            (RowControl::Select { options, index, .. }, ExpectedControl::SelectValue(want)) => {
                assert_eq!(&options[*index].value, want, "{label}");
            }
            (RowControl::TextList(values), ExpectedControl::TextList(want)) => {
                assert_eq!(
                    values.iter().map(String::as_str).collect::<Vec<_>>(),
                    *want,
                    "{label}"
                );
            }
            (RowControl::Color(value), ExpectedControl::Color(want)) => {
                assert_eq!(value, want, "{label}");
            }
            (RowControl::Unsupported { reason, .. }, ExpectedControl::Unsupported(want_reason)) => {
                assert_eq!(reason, want_reason, "{label}");
            }
            (other, want) => panic!("{label}: expected {want:?}, got {other:?}"),
        }
    }

    #[test]
    fn mismatched_stored_values_coerce_or_stay_unsupported() {
        let cases: Vec<(&str, serde_json::Value, ExpectedControl)> = vec![
            (
                "boolean",
                serde_json::json!("true"),
                ExpectedControl::Toggle(true),
            ),
            (
                "boolean",
                serde_json::json!("false"),
                ExpectedControl::Toggle(false),
            ),
            (
                "boolean",
                serde_json::json!("yes"),
                ExpectedControl::Unsupported("boolean field holds a string"),
            ),
            (
                "boolean",
                serde_json::json!(1),
                ExpectedControl::Toggle(true),
            ),
            (
                "boolean",
                serde_json::json!(0),
                ExpectedControl::Toggle(false),
            ),
            (
                "boolean",
                serde_json::json!(2),
                ExpectedControl::Unsupported("boolean field holds a number"),
            ),
            (
                "boolean",
                serde_json::json!(["a"]),
                ExpectedControl::Unsupported("boolean field holds a string_array"),
            ),
            (
                "boolean",
                serde_json::json!([{ "k": true }]),
                ExpectedControl::Unsupported("boolean field holds an object_array"),
            ),
            (
                "boolean",
                serde_json::json!({ "e": { "v": 1 } }),
                ExpectedControl::Unsupported("boolean field holds an object_map"),
            ),
            (
                "number",
                serde_json::json!("5"),
                ExpectedControl::Number(5.0),
            ),
            (
                "number",
                serde_json::json!("5.5"),
                ExpectedControl::Number(5.5),
            ),
            (
                "number",
                serde_json::json!(" 7 "),
                ExpectedControl::Number(7.0),
            ),
            (
                "number",
                serde_json::json!(""),
                ExpectedControl::Unsupported("number field holds a string"),
            ),
            (
                "number",
                serde_json::json!("abc"),
                ExpectedControl::Unsupported("number field holds a string"),
            ),
            (
                "number",
                serde_json::json!(true),
                ExpectedControl::Unsupported("number field holds a boolean"),
            ),
            (
                "number",
                serde_json::json!(["a"]),
                ExpectedControl::Unsupported("number field holds a string_array"),
            ),
            ("string", serde_json::json!(5), ExpectedControl::Text("5")),
            (
                "string",
                serde_json::json!(5.5),
                ExpectedControl::Text("5.5"),
            ),
            (
                "string",
                serde_json::json!(true),
                ExpectedControl::Text("true"),
            ),
            (
                "string",
                serde_json::json!(["a"]),
                ExpectedControl::Unsupported("string field holds a string_array"),
            ),
            (
                "select",
                serde_json::json!(5),
                ExpectedControl::SelectValue("5"),
            ),
            (
                "select",
                serde_json::json!(true),
                ExpectedControl::SelectValue("true"),
            ),
            (
                "select",
                serde_json::json!(["a"]),
                ExpectedControl::Unsupported("select field holds a string_array"),
            ),
            (
                "color",
                serde_json::json!(46657221),
                ExpectedControl::Color("46657221"),
            ),
            (
                "color",
                serde_json::json!(true),
                ExpectedControl::Color("true"),
            ),
            (
                "color",
                serde_json::json!(["a"]),
                ExpectedControl::Unsupported("color field holds a string_array"),
            ),
            (
                "string_array",
                serde_json::json!("foo"),
                ExpectedControl::TextList(vec!["foo"]),
            ),
            (
                "string_array",
                serde_json::json!(5),
                ExpectedControl::Unsupported("string_array field holds a number"),
            ),
            (
                "string_array",
                serde_json::json!(true),
                ExpectedControl::Unsupported("string_array field holds a boolean"),
            ),
            (
                "string_array",
                serde_json::json!([{ "k": true }]),
                ExpectedControl::Unsupported("string_array field holds an object_array"),
            ),
            (
                "object_array",
                serde_json::json!("x"),
                ExpectedControl::Unsupported("object_array field holds a string"),
            ),
            (
                "object_array",
                serde_json::json!(5),
                ExpectedControl::Unsupported("object_array field holds a number"),
            ),
            (
                "object_array",
                serde_json::json!(["a"]),
                ExpectedControl::Unsupported("object_array field holds a string_array"),
            ),
            (
                "object_map",
                serde_json::json!("x"),
                ExpectedControl::Unsupported("object_map field holds a string"),
            ),
            (
                "object_map",
                serde_json::json!(5),
                ExpectedControl::Unsupported("object_map field holds a number"),
            ),
            (
                "object_map",
                serde_json::json!(["a"]),
                ExpectedControl::Unsupported("object_map field holds a string_array"),
            ),
            (
                "object_map",
                serde_json::json!(true),
                ExpectedControl::Unsupported("object_map field holds a boolean"),
            ),
        ];
        for (kind, overrides, expected) in cases {
            let label = format!("{kind} override {overrides}");
            let spec = qol_config::contract::parse_spec_str(&coercion_spec(kind)).unwrap();
            let resolved = qol_config::normalized::resolve_config(
                &spec,
                &serde_json::json!({ "value": overrides }),
            )
            .unwrap_or_else(|errors| panic!("{label}: {errors:?}"));
            let rows = rows_from_resolved(&resolved);
            assert_eq!(rows.len(), 1, "{label}");
            assert_expected_control(&rows, &label, &expected);
        }
    }

    #[test]
    fn stream_rows_report_streams_and_actions_from_the_contract() {
        const STREAM_SPEC: &str = r##"
schema_version = 1

[field.live_color]
type = "color"
config_key = "live_color_hex"
label = "Color"
default = "#FFFFFF"
stream = "live_color"
action = "set_color_main"

[field.live_brightness]
type = "number"
config_key = "live_brightness"
label = "Brightness"
default = 100
min = 1
max = 100
step = 1
variant = "slider"
stream = "live_color"
action = "set_brightness_main"

[field.plain]
type = "number"
config_key = "plain"
label = "Plain"
default = 5
"##;
        let spec = qol_config::contract::parse_spec_str(STREAM_SPEC).unwrap();
        let resolved =
            qol_config::normalized::resolve_config(&spec, &serde_json::json!({})).unwrap();
        let rows = rows_from_resolved(&resolved);

        assert!(row_streams(&rows, 0));
        assert_eq!(row_action(&rows, 0).as_deref(), Some("set_color_main"));
        assert!(row_streams(&rows, 1));
        assert_eq!(row_action(&rows, 1).as_deref(), Some("set_brightness_main"));
        assert!(!row_streams(&rows, 2));
        assert_eq!(row_action(&rows, 2), None);
    }

    #[test]
    fn stream_gating_follows_status_rows_reporting_danger_or_error() {
        fn status_row(tone: StatusTone, error: Option<&str>) -> Row {
            Row {
                id: "status".into(),
                section_id: None,
                section_label: None,
                label: "Status".into(),
                description: None,
                placeholder: None,
                variant: None,
                config_key: "status".into(),
                default: FieldDefault::Boolean(false),
                stream: None,
                action: None,
                visibility: None,
                control: RowControl::Status {
                    query: "runtime".into(),
                    value_from: None,
                    label_map: std::collections::BTreeMap::new(),
                    tone_map: std::collections::BTreeMap::new(),
                    label: None,
                    tone,
                    error: error.map(str::to_string),
                },
            }
        }

        let neutral = vec![status_row(StatusTone::Muted, None)];
        assert!(!stream_gated(&neutral));
        let danger = vec![status_row(StatusTone::Danger, None)];
        assert!(stream_gated(&danger));
        let errored = vec![status_row(StatusTone::Muted, Some("query failed"))];
        assert!(stream_gated(&errored));
    }

    #[test]
    fn list_filter_matches_web_semantics() {
        let actions = ListActions {
            primary: Some(qol_config::contract::RowActionSpec {
                action: "connect".into(),
                input: None,
                label: Some("Connect".into()),
                key: None,
                when: Some("can_connect".into()),
            }),
            additional: vec![qol_config::contract::RowActionSpec {
                action: "remove".into(),
                input: None,
                label: Some("Remove".into()),
                key: None,
                when: None,
            }],
        };
        let items = vec![
            filter_item("WH-1000XM4", Some("AA:BB:CC:DD:EE:FF"), true),
            filter_item("MX Master 3", Some("11:22:33:44:55:66"), false),
            filter_item("Keyboard", None, true),
        ];
        let cases = [
            ("", vec![0, 1, 2], "empty query keeps all"),
            ("   ", vec![0, 1, 2], "whitespace query keeps all"),
            ("wh-1000", vec![0], "case-insensitive label match"),
            ("mx", vec![1], "label fragment match"),
            ("55:66", vec![1], "subtitle match"),
            ("connect", vec![0, 2], "when-gated action label match"),
            ("remove", vec![0, 1, 2], "ungated action label match"),
            ("zzz", vec![], "no match yields the empty set"),
        ];
        for (query, expected, label) in cases {
            assert_eq!(
                filtered_list_items(&actions, &items, query),
                expected,
                "{label}: query {query:?}"
            );
        }
    }

    #[test]
    fn list_filter_resets_selection_to_the_first_visible_item() {
        let actions = ListActions {
            primary: None,
            additional: Vec::new(),
        };
        let items = (0..12)
            .map(|index| filter_item(&format!("device {index}"), None, false))
            .collect::<Vec<_>>();
        let mut list = crate::scroll_list::ScrollList::new(super::LIST_MAX_VISIBLE);
        list.selected = 9;
        list.scroll_offset = 5;
        let mut filter = String::new();

        apply_list_filter(&actions, &items, &mut list, &mut filter, "device 1".into());

        assert_eq!(filter, "device 1");
        assert_eq!(filtered_list_items(&actions, &items, &filter), [1, 10, 11]);
        assert_eq!(
            (list.selected, list.scroll_offset),
            (0, 0),
            "selection lands on the first visible item"
        );

        apply_list_filter(&actions, &items, &mut list, &mut filter, String::new());
        assert_eq!((list.selected, list.scroll_offset), (0, 0));
    }

    #[test]
    fn render_template_matches_the_web_interpolator() {
        let row = serde_json::json!({
            "name": "WH-1000XM4",
            "index": 5,
            "paired": true,
            "nulled": null,
        });
        let cases = [
            ("{name}", "WH-1000XM4", "key present as string renders bare"),
            (
                "{index}",
                "5",
                "key present as number renders its JSON form",
            ),
            (
                "{paired}",
                "true",
                "key present as bool renders its JSON form",
            ),
            ("{nulled}", "", "key present as null renders nothing"),
            ("{missing}", "", "key absent entirely renders nothing"),
            (
                "{name} on {missing}",
                "WH-1000XM4 on ",
                "only the resolvable placeholder is replaced",
            ),
            (
                "{not a key}",
                "{not a key}",
                "a non-word brace expression is left verbatim",
            ),
            ("{}", "{}", "an empty brace pair is left verbatim"),
            (
                "plain text",
                "plain text",
                "a template without placeholders is unchanged",
            ),
        ];
        for (template, expected, label) in cases {
            assert_eq!(
                render_template(template, &row),
                expected,
                "{label}: template {template:?}"
            );
        }
    }

    #[test]
    fn render_template_hides_fully_unresolvable_subtitles() {
        let row = serde_json::json!({ "name": "Keyboard" });
        let items = list_items(
            &serde_json::json!({ "items": [row.clone()] }),
            "{name}",
            Some("{detail}"),
        );
        assert_eq!(items[0].label, "Keyboard");
        assert_eq!(
            items[0].subtitle, None,
            "an unresolved subtitle hides the line"
        );
    }

    fn filter_item(label: &str, subtitle: Option<&str>, can_connect: bool) -> ListItem {
        ListItem {
            id: label.into(),
            label: label.into(),
            subtitle: subtitle.map(str::to_string),
            accent: None,
            badge: None,
            badge_tone: None,
            data: serde_json::json!({ "can_connect": can_connect }),
            pending: false,
            error: None,
        }
    }
}
