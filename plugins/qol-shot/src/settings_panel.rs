use qol_config::contract::{FieldDefault, FieldKind};
use qol_config::normalized::{ResolvedConfig, ResolvedField, ResolvedSection};

#[derive(Debug)]
pub(crate) enum RowControl {
    Toggle(bool),
    Select {
        options: Vec<String>,
        labels: Vec<String>,
        index: usize,
    },
    Number {
        value: f64,
        min: Option<f64>,
        max: Option<f64>,
        step: f64,
    },
    Text(String),
    TextList(Vec<String>),
}

pub(crate) struct Row {
    pub(crate) section_label: Option<String>,
    pub(crate) label: String,
    pub(crate) config_key: String,
    pub(crate) control: RowControl,
}

pub(crate) fn rows_from_resolved(config: &ResolvedConfig) -> Vec<Row> {
    let mut rows = Vec::new();
    for field in &config.fields {
        push_row(&mut rows, None, field);
    }
    for section in &config.sections {
        push_section_rows(&mut rows, section);
    }
    rows
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
            let index = field.options.iter().position(|o| *o == current)?;
            let labels = field
                .options
                .iter()
                .map(|o| {
                    field
                        .option_labels
                        .get(o)
                        .cloned()
                        .unwrap_or_else(|| o.clone())
                })
                .collect();
            Some(RowControl::Select {
                options: field.options.clone(),
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
            FieldDefault::StringArray(values) => Some(RowControl::TextList(values.clone())),
            _ => None,
        },
        FieldKind::ObjectArray
        | FieldKind::ObjectMap
        | FieldKind::Color
        | FieldKind::Action
        | FieldKind::List
        | FieldKind::Status
        | FieldKind::QrCode
        | FieldKind::Gamepad => None,
    }
}

pub(crate) fn row_value_json(control: &RowControl) -> serde_json::Value {
    match control {
        RowControl::Toggle(value) => serde_json::json!(value),
        RowControl::Select { options, index, .. } => serde_json::json!(options[*index]),
        RowControl::Number { value, .. } => serde_json::json!(value),
        RowControl::Text(value) => serde_json::json!(value),
        RowControl::TextList(values) => serde_json::json!(values),
    }
}

pub(crate) fn set_config_value(
    root: &mut serde_json::Value,
    dotted_key: &str,
    value: serde_json::Value,
) {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Intent {
    Up,
    Down,
    Toggle,
    Left,
    Right,
    BeginEdit,
    CommitEdit,
    Backspace,
    Insert(String),
    Close,
    CancelEdit,
}

pub(crate) fn intent(key: &str, key_char: Option<&str>, editing: bool) -> Option<Intent> {
    if editing {
        return match key {
            "enter" => Some(Intent::CommitEdit),
            "escape" => Some(Intent::CancelEdit),
            "backspace" => Some(Intent::Backspace),
            _ => key_char.map(|ch| Intent::Insert(ch.to_string())),
        };
    }
    match key {
        "up" => Some(Intent::Up),
        "down" => Some(Intent::Down),
        "space" => Some(Intent::Toggle),
        "left" => Some(Intent::Left),
        "right" => Some(Intent::Right),
        "enter" => Some(Intent::BeginEdit),
        "escape" => Some(Intent::Close),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(rows.len(), 5);
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
        assert!(
            matches!(&rows[4].control, RowControl::TextList(v) if v == &vec!["mic".to_string()])
        );
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
    fn intent_maps_navigation_editing_and_close() {
        let cases = [
            ("up", None, false, Some(Intent::Up)),
            ("down", None, false, Some(Intent::Down)),
            ("space", None, false, Some(Intent::Toggle)),
            ("left", None, false, Some(Intent::Left)),
            ("right", None, false, Some(Intent::Right)),
            ("enter", None, false, Some(Intent::BeginEdit)),
            ("escape", None, false, Some(Intent::Close)),
            ("enter", None, true, Some(Intent::CommitEdit)),
            ("escape", None, true, Some(Intent::CancelEdit)),
            ("backspace", None, true, Some(Intent::Backspace)),
            ("a", Some("a"), true, Some(Intent::Insert("a".into()))),
            ("a", Some("a"), false, None),
        ];
        for (key, ch, editing, expected) in cases {
            assert_eq!(
                intent(key, ch, editing),
                expected,
                "key {key} editing {editing}"
            );
        }
    }
}
