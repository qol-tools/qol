use qol_config::contract::{FieldDefault, FieldKind};
use qol_config::normalized::{ResolvedConfig, ResolvedField, ResolvedSection};

use super::SettingsRuntime;
use crate::scroll_list::ScrollList;

pub(super) const LIST_MAX_VISIBLE: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ListItem {
    pub(super) label: String,
    pub(super) subtitle: Option<String>,
    pub(super) data: serde_json::Value,
}

#[derive(Debug)]
pub(super) enum RowControl {
    Toggle(bool),
    Select {
        options: Vec<String>,
        labels: Vec<String>,
        index: usize,
    },
    MultiSelect {
        options: Vec<String>,
        labels: Vec<String>,
        selected: Vec<bool>,
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
        active: bool,
        pending: bool,
        error: Option<String>,
    },
    List {
        query: String,
        row_label: String,
        row_subtitle: Option<String>,
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

pub(super) fn rows_from_resolved(config: &ResolvedConfig, runtime: &SettingsRuntime) -> Vec<Row> {
    let mut rows = Vec::new();
    for field in &config.fields {
        push_row(&mut rows, None, field, runtime);
    }
    for section in &config.sections {
        push_section_rows(&mut rows, section, runtime);
    }
    rows
}

fn push_section_rows(rows: &mut Vec<Row>, section: &ResolvedSection, runtime: &SettingsRuntime) {
    let mut label = Some(section.label.clone());
    for field in &section.fields {
        let before = rows.len();
        push_row(rows, label.clone(), field, runtime);
        if rows.len() > before {
            label = None;
        }
    }
}

fn push_row(
    rows: &mut Vec<Row>,
    section_label: Option<String>,
    field: &ResolvedField,
    runtime: &SettingsRuntime,
) {
    let Some(control) = control_for(field, runtime) else {
        return;
    };
    rows.push(Row {
        section_label,
        label: field.label.clone(),
        config_key: field.config_key.clone(),
        control,
    });
}

fn control_for(field: &ResolvedField, runtime: &SettingsRuntime) -> Option<RowControl> {
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
            let (options, labels) = field_options(field, std::slice::from_ref(&current), runtime);
            let index = options.iter().position(|o| *o == current)?;
            Some(RowControl::Select {
                options,
                labels,
                index,
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
                    let (options, labels) = field_options(field, values, runtime);
                    Some(RowControl::MultiSelect {
                        selected: options
                            .iter()
                            .map(|option| values.contains(option))
                            .collect(),
                        options,
                        labels,
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
            active: false,
            pending: false,
            error: None,
        }),
        FieldKind::List => field.query.clone().map(|query| RowControl::List {
            query,
            row_label: field.row_label.clone().unwrap_or_else(|| "{name}".into()),
            row_subtitle: field.row_subtitle.clone(),
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
    runtime: &SettingsRuntime,
) -> (Vec<String>, Vec<String>) {
    let dynamic = field
        .query
        .as_deref()
        .and_then(|query| runtime.query(query).ok())
        .map(options_from_value)
        .unwrap_or_default();
    let mut options = field.options.clone();
    if field.query.is_some() {
        for option in field.option_labels.keys() {
            if !options.iter().any(|candidate| candidate == option) {
                options.push(option.clone());
            }
        }
    }
    for (value, _) in &dynamic {
        if !options.iter().any(|option| option == value) {
            options.push(value.clone());
        }
    }
    for value in current.iter().rev() {
        if !options.iter().any(|option| option == value) {
            options.insert(0, value.clone());
        }
    }
    let labels = options
        .iter()
        .map(|option| {
            field
                .option_labels
                .get(option)
                .cloned()
                .or_else(|| {
                    dynamic
                        .iter()
                        .find(|(value, _)| value == option)
                        .map(|(_, label)| label.clone())
                })
                .unwrap_or_else(|| option.clone())
        })
        .collect();
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
            RowControl::Action {
                active_query: Some(query),
                ..
            }
            | RowControl::List { query, .. } => {
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
                row_label,
                row_subtitle,
                items,
                list,
                error,
                ..
            } if row_query == query => match &result {
                Ok(value) => {
                    *items = list_items(value, row_label, row_subtitle.as_deref());
                    list.sync(items.len());
                    *error = None;
                }
                Err(message) => *error = Some(message.clone()),
            },
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
        .map(|row| ListItem {
            label: render_template(label, row),
            subtitle: subtitle
                .map(|template| render_template(template, row))
                .filter(|text| !text.is_empty()),
            data: row.clone(),
        })
        .collect()
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

fn options_from_value(value: serde_json::Value) -> Vec<(String, String)> {
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
        apply_runtime_query, merged_config, row_value_json, rows_from_resolved, set_config_value,
        ResolvedConfig, Row, RowControl,
    };
    use crate::settings_panel::SettingsRuntime;

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
        let rows = rows_from_resolved(
            &resolved(serde_json::json!({
                "capture": { "pin_border": false, "saved_feedback": "toast" }
            })),
            &SettingsRuntime::empty(),
        );
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
        let rows = rows_from_resolved(&resolved, &SettingsRuntime::empty());

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
        let runtime = SettingsRuntime::new(|query: &str| {
            assert_eq!(query, "audio_sources");
            Ok(serde_json::json!([
                { "value": "alsa_input.foo", "label": "Built-in Mic" }
            ]))
        });

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
            let rows = rows_from_resolved(&resolved, &runtime);
            match &rows[0].control {
                RowControl::Select {
                    options,
                    labels,
                    index,
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
        let runtime = SettingsRuntime::new(|query| {
            assert_eq!(query, "managed_device_options");
            Ok(serde_json::json!([
                { "value": "AA:01", "label": "Headphones · AA:01" },
                { "value": "AA:02", "label": "Keyboard · AA:02" }
            ]))
        });
        let rows = rows_from_resolved(&resolved, &runtime);

        match &rows[0].control {
            RowControl::MultiSelect {
                options,
                labels,
                selected,
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
        };
        assert_eq!(
            row_value_json(&control),
            Some(serde_json::json!(["system"]))
        );
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
row_label = "{name}"
row_subtitle = "{detail}"
empty_message = "No devices."
"#;
        let spec = qol_config::contract::parse_spec_str(RUNTIME_SPEC).unwrap();
        let resolved =
            qol_config::normalized::resolve_config(&spec, &serde_json::json!({})).unwrap();
        let mut rows = rows_from_resolved(&resolved, &SettingsRuntime::empty());

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
                    { "name": "Keyboard", "detail": "Paired · Connected" },
                    { "name": "Headphones", "detail": "Discovered" }
                ]
            })),
        );

        assert!(matches!(
            rows[0].control,
            RowControl::Action { active: true, .. }
        ));
        match &rows[1].control {
            RowControl::List { items, .. } => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0].label, "Keyboard");
                assert_eq!(items[0].subtitle.as_deref(), Some("Paired · Connected"));
                assert_eq!(items[1].label, "Headphones");
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
