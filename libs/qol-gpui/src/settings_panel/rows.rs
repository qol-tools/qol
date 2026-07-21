use qol_config::contract::{
    resolve_row_actions, FieldDefault, FieldKind, ResolvedRowAction, RowActionSpec,
};
use qol_config::normalized::{ResolvedConfig, ResolvedField, ResolvedSection};

use crate::scroll_list::ScrollList;
use crate::status_indicator::StatusTone;

use super::SettingsCatalogRow;

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
    seeded: Vec<(String, String)>,
}

#[derive(Debug)]
pub(super) enum RowControl {
    Toggle(bool),
    Select {
        options: Vec<String>,
        labels: Vec<String>,
        index: usize,
        dynamic: Option<OptionQuery>,
    },
    MultiSelect {
        options: Vec<String>,
        labels: Vec<String>,
        selected: Vec<bool>,
        dynamic: Option<OptionQuery>,
    },
    Number {
        value: f64,
        min: Option<f64>,
        max: Option<f64>,
        step: f64,
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
        variant: Option<String>,
        active: bool,
        pending: bool,
        error: Option<String>,
    },
    List {
        query: String,
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
}

pub(super) struct Row {
    pub(super) section_label: Option<String>,
    pub(super) label: String,
    pub(super) config_key: String,
    pub(super) control: RowControl,
}

pub(super) fn rows_from_resolved(config: &ResolvedConfig) -> Vec<Row> {
    let mut rows = Vec::new();
    for field in &config.fields {
        push_row(&mut rows, None, field);
    }
    for section in &config.sections {
        push_section_rows(&mut rows, section);
    }
    rows
}

pub(super) fn catalog_rows(config: &ResolvedConfig) -> Vec<SettingsCatalogRow> {
    let mut rows = Vec::new();
    for field in &config.fields {
        push_catalog_row(&mut rows, None, field);
    }
    for section in &config.sections {
        for field in &section.fields {
            push_catalog_row(&mut rows, Some(section.label.clone()), field);
        }
    }
    rows
}

fn push_catalog_row(
    rows: &mut Vec<SettingsCatalogRow>,
    section: Option<String>,
    field: &ResolvedField,
) {
    if control_for(field).is_none() {
        return;
    }
    rows.push(SettingsCatalogRow {
        config_key: field.config_key.clone(),
        section,
        label: field.label.clone(),
    });
}

fn push_section_rows(rows: &mut Vec<Row>, section: &ResolvedSection) {
    let mut label = Some(section.label.clone());
    for field in &section.fields {
        let before = rows.len();
        push_row(rows, label.clone(), field);
        if rows.len() > before {
            label = None;
        }
    }
}

fn push_row(rows: &mut Vec<Row>, section_label: Option<String>, field: &ResolvedField) {
    let Some(control) = control_for(field) else {
        return;
    };
    rows.push(Row {
        section_label,
        label: field.label.clone(),
        config_key: field.config_key.clone(),
        control,
    });
}

fn control_for(field: &ResolvedField) -> Option<RowControl> {
    match field.kind {
        FieldKind::Boolean => match field.value {
            FieldDefault::Boolean(value) => Some(RowControl::Toggle(value)),
            _ => None,
        },
        FieldKind::Select => {
            let current = match &field.value {
                FieldDefault::String(value) => value.clone(),
                _ => return None,
            };
            let (options, labels, dynamic) = field_options(field, std::slice::from_ref(&current));
            let index = options.iter().position(|o| *o == current)?;
            Some(RowControl::Select {
                options,
                labels,
                index,
                dynamic,
            })
        }
        FieldKind::Number => match field.value {
            FieldDefault::Number(value) => Some(RowControl::Number {
                value,
                min: field.number.min,
                max: field.number.max,
                step: field.number.step.unwrap_or(1.0),
            }),
            _ => None,
        },
        FieldKind::String => match &field.value {
            FieldDefault::String(value) => Some(RowControl::Text(value.clone())),
            _ => None,
        },
        FieldKind::StringArray => match &field.value {
            FieldDefault::StringArray(values) => {
                if field.options.is_empty() && field.query.is_none() {
                    Some(RowControl::TextList(values.clone()))
                } else {
                    let (options, labels, dynamic) = field_options(field, values);
                    Some(RowControl::MultiSelect {
                        selected: options
                            .iter()
                            .map(|option| values.contains(option))
                            .collect(),
                        options,
                        labels,
                        dynamic,
                    })
                }
            }
            _ => None,
        },
        FieldKind::Color => match &field.value {
            FieldDefault::String(value) => Some(RowControl::Color(value.clone())),
            _ => None,
        },
        FieldKind::Action => field.action.clone().map(|action| RowControl::Action {
            action,
            active_action: field.active_action.clone(),
            active_label: field.active_label.clone(),
            active_query: field.active_query.clone(),
            active_value_from: field.active_value_from.clone(),
            variant: field.variant.clone(),
            active: false,
            pending: false,
            error: None,
        }),
        FieldKind::List => field.query.clone().map(|query| RowControl::List {
            query,
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
        }),
        FieldKind::ObjectArray
        | FieldKind::ObjectMap
        | FieldKind::Status
        | FieldKind::QrCode
        | FieldKind::Gamepad => None,
    }
}

fn field_options(
    field: &ResolvedField,
    current: &[String],
) -> (Vec<String>, Vec<String>, Option<OptionQuery>) {
    let seeded = seeded_options(field);
    let (options, labels) = merge_options(&seeded, &[], current);
    let dynamic = field.query.as_ref().map(|name| OptionQuery {
        name: name.clone(),
        seeded,
    });
    (options, labels, dynamic)
}

fn seeded_options(field: &ResolvedField) -> Vec<(String, String)> {
    let mut seeded = field
        .options
        .iter()
        .map(|option| {
            let label = field
                .option_labels
                .get(option)
                .cloned()
                .unwrap_or_else(|| option.clone());
            (option.clone(), label)
        })
        .collect::<Vec<_>>();
    if field.query.is_some() {
        for (option, label) in &field.option_labels {
            if !seeded.iter().any(|(candidate, _)| candidate == option) {
                seeded.push((option.clone(), label.clone()));
            }
        }
    }
    seeded
}

fn merge_options(
    seeded: &[(String, String)],
    dynamic: &[(String, String)],
    current: &[String],
) -> (Vec<String>, Vec<String>) {
    let mut merged = seeded.to_vec();
    for (value, label) in dynamic {
        if !merged.iter().any(|(option, _)| option == value) {
            merged.push((value.clone(), label.clone()));
        }
    }
    for value in current.iter().rev() {
        if !merged.iter().any(|(option, _)| option == value) {
            merged.insert(0, (value.clone(), value.clone()));
        }
    }
    let (options, labels) = merged.into_iter().unzip();
    (options, labels)
}

pub(super) fn merged_config(base: &serde_json::Value, rows: &[Row]) -> serde_json::Value {
    let mut config = if base.is_object() {
        base.clone()
    } else {
        serde_json::json!({})
    };
    for row in rows {
        if let Some(value) = row_value_json(&row.control) {
            set_config_value(&mut config, &row.config_key, value);
        }
    }
    config
}

fn row_value_json(control: &RowControl) -> Option<serde_json::Value> {
    match control {
        RowControl::Toggle(value) => Some(serde_json::json!(value)),
        RowControl::Select { options, index, .. } => Some(serde_json::json!(options[*index])),
        RowControl::MultiSelect {
            options, selected, ..
        } => {
            let values: Vec<&String> = options
                .iter()
                .zip(selected)
                .filter(|(_, on)| **on)
                .map(|(option, _)| option)
                .collect();
            Some(serde_json::json!(values))
        }
        RowControl::Number { value, .. } => Some(number_json(*value)),
        RowControl::Text(value) => Some(serde_json::json!(value)),
        RowControl::TextList(values) => Some(serde_json::json!(values)),
        RowControl::Color(value) => Some(serde_json::json!(value)),
        RowControl::Action { .. } | RowControl::List { .. } => None,
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
            RowControl::List {
                query,
                active_query,
                ..
            } => {
                names.insert(query.clone());
                names.extend(active_query.iter().cloned());
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
                labels,
                index,
                dynamic: Some(dynamic),
            } if dynamic.name == query => {
                if let Ok(value) = &result {
                    let current = options.get(*index).cloned().unwrap_or_default();
                    let fetched = options_from_value(value);
                    let (next_options, next_labels) =
                        merge_options(&dynamic.seeded, &fetched, std::slice::from_ref(&current));
                    *index = next_options
                        .iter()
                        .position(|option| option == &current)
                        .unwrap_or(0);
                    *options = next_options;
                    *labels = next_labels;
                }
            }
            RowControl::MultiSelect {
                options,
                labels,
                selected,
                dynamic: Some(dynamic),
            } if dynamic.name == query => {
                if let Ok(value) = &result {
                    let current = options
                        .iter()
                        .zip(selected.iter())
                        .filter(|(_, selected)| **selected)
                        .map(|(option, _)| option.clone())
                        .collect::<Vec<_>>();
                    let fetched = options_from_value(value);
                    let (next_options, next_labels) =
                        merge_options(&dynamic.seeded, &fetched, &current);
                    *selected = next_options
                        .iter()
                        .map(|option| current.contains(option))
                        .collect();
                    *options = next_options;
                    *labels = next_labels;
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
            _ => {}
        }
    }
}

fn query_flag(value: &serde_json::Value, path: Option<&str>) -> bool {
    let selected = path
        .filter(|path| !path.is_empty())
        .and_then(|path| path.split('.').try_fold(value, |value, key| value.get(key)))
        .unwrap_or(value);
    selected.as_bool().unwrap_or(false)
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

fn render_template(template: &str, row: &serde_json::Value) -> String {
    let mut rendered = template.to_string();
    if let Some(fields) = row.as_object() {
        for (key, value) in fields {
            let replacement = value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string());
            rendered = rendered.replace(&format!("{{{key}}}"), &replacement);
        }
    }
    rendered
}

fn options_from_value(value: &serde_json::Value) -> Vec<(String, String)> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            if let Some(value) = item.as_str() {
                return Some((value.to_string(), value.to_string()));
            }
            let value = item.get("value")?.as_str()?.to_string();
            let label = item
                .get("label")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(&value)
                .to_string();
            Some((value, label))
        })
        .collect()
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
        apply_runtime_query, begin_list_item_action, list_item_actions, list_items, merged_config,
        primary_list_item_action, row_value_json, rows_from_resolved, runtime_query_names,
        set_config_value, ResolvedConfig, Row, RowControl,
    };
    use crate::status_indicator::StatusTone;

    const SPEC: &str = r#"
schema_version = 1

[section.capture]
label = "Capture"

[field.pin_border]
type = "boolean"
config_key = "capture.pin_border"
label = "Pinned Preview Border"
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

[field.mic]
type = "string"
config_key = "audio.mic_device"
label = "Mic Device"
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

    #[test]
    fn rows_map_every_supported_kind_with_override_values() {
        let rows = rows_from_resolved(&resolved(serde_json::json!({
            "capture": { "pin_border": false, "saved_feedback": "toast" }
        })));
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[0].section_label.as_deref(), Some("Capture"));
        assert!(rows[1..].iter().all(|r| r.section_label.is_none()));
        assert!(matches!(rows[0].control, RowControl::Toggle(false)));
        match &rows[1].control {
            RowControl::Select { index, options, .. } => {
                assert_eq!(options[*index], "toast");
            }
            other => panic!("expected select, got {other:?}"),
        }
        match &rows[2].control {
            RowControl::Number { value, step, .. } => {
                assert_eq!((*value, *step), (18.0, 1.0));
            }
            other => panic!("expected number, got {other:?}"),
        }
        assert!(matches!(&rows[3].control, RowControl::Text(v) if v == "default"));
        match &rows[4].control {
            RowControl::MultiSelect {
                options,
                labels,
                selected,
                ..
            } => {
                assert_eq!(options, &["mic", "system"]);
                assert_eq!(labels, &["Microphone", "System Audio"]);
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
                    { "value": "alsa_input.foo", "label": "Built-in Mic" }
                ])),
            );
            match &rows[0].control {
                RowControl::Select {
                    options,
                    labels,
                    index,
                    ..
                } => {
                    assert_eq!(options, &expected_options, "overrides: {overrides}");
                    assert_eq!(labels, &expected_labels, "overrides: {overrides}");
                    assert_eq!(*index, expected_index, "overrides: {overrides}");
                }
                other => panic!("expected select, got {other:?}"),
            }
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
            RowControl::Select { options, .. } => assert_eq!(options, &["default"]),
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
                options,
                labels,
                selected,
                ..
            } => {
                assert_eq!(options, &["AA:00", "AA:01", "AA:02"]);
                assert_eq!(labels, &["AA:00", "Headphones · AA:01", "Keyboard · AA:02"]);
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
                section_label: None,
                label: "Action Mode".into(),
                config_key: "action_mode".into(),
                control: RowControl::Select {
                    options: vec!["hold_to_switch".into(), "sticky".into()],
                    labels: vec!["Hold".into(), "Sticky".into()],
                    index: 1,
                    dynamic: None,
                },
            },
            Row {
                section_label: None,
                label: "Max Columns".into(),
                config_key: "display.max_columns".into(),
                control: RowControl::Number {
                    value: 4.0,
                    min: None,
                    max: None,
                    step: 1.0,
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
                step: 1.0,
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
            options: vec!["mic".into(), "system".into()],
            labels: vec!["Microphone".into(), "System Audio".into()],
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
    fn runtime_rows_are_not_written_into_plugin_config() {
        let rows = vec![Row {
            section_label: None,
            label: "Start search".into(),
            config_key: "search".into(),
            control: RowControl::Action {
                action: "start_search".into(),
                active_action: None,
                active_label: None,
                active_query: None,
                active_value_from: None,
                variant: None,
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
}
