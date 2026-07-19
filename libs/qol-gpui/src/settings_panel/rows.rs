use qol_config::contract::{FieldDefault, FieldKind};
use qol_config::normalized::{ResolvedConfig, ResolvedField, ResolvedSection};

use super::QueryOptions;

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
}

pub(super) struct Row {
    pub(super) section_label: Option<String>,
    pub(super) label: String,
    pub(super) config_key: String,
    pub(super) control: RowControl,
}

pub(super) fn rows_from_resolved(config: &ResolvedConfig, provider: &QueryOptions) -> Vec<Row> {
    let mut rows = Vec::new();
    for field in &config.fields {
        push_row(&mut rows, None, field, provider);
    }
    for section in &config.sections {
        push_section_rows(&mut rows, section, provider);
    }
    rows
}

fn push_section_rows(rows: &mut Vec<Row>, section: &ResolvedSection, provider: &QueryOptions) {
    let mut label = Some(section.label.clone());
    for field in &section.fields {
        let before = rows.len();
        push_row(rows, label.clone(), field, provider);
        if rows.len() > before {
            label = None;
        }
    }
}

fn push_row(
    rows: &mut Vec<Row>,
    section_label: Option<String>,
    field: &ResolvedField,
    provider: &QueryOptions,
) {
    let Some(control) = control_for(field, provider) else {
        return;
    };
    rows.push(Row {
        section_label,
        label: field.label.clone(),
        config_key: field.config_key.clone(),
        control,
    });
}

fn control_for(field: &ResolvedField, provider: &QueryOptions) -> Option<RowControl> {
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
            let (options, labels) = field_options(field, std::slice::from_ref(&current), provider);
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
                    let (options, labels) = field_options(field, values, provider);
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
        FieldKind::ObjectArray
        | FieldKind::ObjectMap
        | FieldKind::Action
        | FieldKind::List
        | FieldKind::Status
        | FieldKind::QrCode
        | FieldKind::Gamepad => None,
    }
}

fn field_options(
    field: &ResolvedField,
    current: &[String],
    provider: &QueryOptions,
) -> (Vec<String>, Vec<String>) {
    let dynamic = field.query.as_deref().map(provider).unwrap_or_default();
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
        set_config_value(&mut config, &row.config_key, row_value_json(&row.control));
    }
    config
}

fn row_value_json(control: &RowControl) -> serde_json::Value {
    match control {
        RowControl::Toggle(value) => serde_json::json!(value),
        RowControl::Select { options, index, .. } => serde_json::json!(options[*index]),
        RowControl::MultiSelect {
            options, selected, ..
        } => {
            let values: Vec<&String> = options
                .iter()
                .zip(selected)
                .filter(|(_, on)| **on)
                .map(|(option, _)| option)
                .collect();
            serde_json::json!(values)
        }
        RowControl::Number { value, .. } => number_json(*value),
        RowControl::Text(value) => serde_json::json!(value),
        RowControl::TextList(values) => serde_json::json!(values),
        RowControl::Color(value) => serde_json::json!(value),
    }
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
        merged_config, row_value_json, rows_from_resolved, set_config_value, ResolvedConfig, Row,
        RowControl,
    };

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
            &|_| Vec::new(),
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
    fn unsupported_action_and_list_fields_do_not_hide_later_supported_sections() {
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
        let rows = rows_from_resolved(&resolved, &|_| Vec::new());

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].section_label.as_deref(), Some("Reconnection"));
        assert_eq!(rows[0].config_key, "auto_reconnect");
        assert!(matches!(rows[0].control, RowControl::Toggle(true)));
        assert_eq!(rows[1].section_label, None);
        assert_eq!(rows[1].config_key, "retry_initial_seconds");
        assert!(matches!(
            rows[1].control,
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
        let provider = |query: &str| {
            assert_eq!(query, "audio_sources");
            vec![("alsa_input.foo".to_string(), "Built-in Mic".to_string())]
        };

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
            let rows = rows_from_resolved(&resolved, &provider);
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
        let rows = rows_from_resolved(&resolved, &|query| {
            assert_eq!(query, "managed_device_options");
            vec![
                ("AA:01".into(), "Headphones · AA:01".into()),
                ("AA:02".into(), "Keyboard · AA:02".into()),
            ]
        });

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
            assert_eq!(row_value_json(&control), expected, "value: {value}");
        }
    }

    #[test]
    fn color_value_json_keeps_the_stored_string() {
        let control = RowControl::Color("#202322".into());
        assert_eq!(row_value_json(&control), serde_json::json!("#202322"));
    }

    #[test]
    fn multi_select_value_json_preserves_option_order() {
        let control = RowControl::MultiSelect {
            options: vec!["mic".into(), "system".into()],
            labels: vec!["Microphone".into(), "System Audio".into()],
            selected: vec![false, true],
        };
        assert_eq!(row_value_json(&control), serde_json::json!(["system"]));
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
