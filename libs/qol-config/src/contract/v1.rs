use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub type ConfigSpec = ConfigSpecV1;

#[derive(Debug)]
pub enum ParseSpecError {
    Io(std::io::Error),
    Toml(toml::de::Error),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ConfigSpecV1 {
    pub schema_version: u32,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "section")]
    pub sections: IndexMap<String, SectionSpec>,
    #[serde(default, rename = "field")]
    pub fields: IndexMap<String, FieldSpec>,
}

impl ConfigSpecV1 {
    pub fn field(&self, id: &str) -> Option<&FieldSpec> {
        self.fields.get(id)
    }

    pub fn section(&self, id: &str) -> Option<&SectionSpec> {
        self.sections.get(id)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct SectionSpec {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub actions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct FieldSpec {
    #[serde(rename = "type")]
    pub kind: FieldKind,
    #[serde(default)]
    pub config_key: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub placeholder: Option<String>,
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default)]
    pub default: Option<FieldDefault>,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub option_labels: IndexMap<String, String>,
    #[serde(default)]
    pub key_label: Option<String>,
    #[serde(default)]
    pub item: Option<ItemSpec>,
    #[serde(default)]
    pub entry_fields: IndexMap<String, FieldKind>,
    #[serde(default)]
    pub show_when: Option<ShowWhenSpec>,
    #[serde(default)]
    pub alpha: Option<bool>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub row_label: Option<String>,
    #[serde(default)]
    pub row_subtitle: Option<String>,
    #[serde(default)]
    pub empty_message: Option<String>,
    #[serde(default)]
    pub value_from: Option<String>,
    #[serde(default)]
    pub label_map: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub tone_map: Option<IndexMap<String, String>>,
    #[serde(flatten)]
    pub number: NumberConstraints,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ShowWhenSpec {
    pub field: String,
    pub equals: FieldDefault,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct ItemSpec {
    #[serde(default)]
    pub fields: IndexMap<String, FieldKind>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FieldKind {
    Boolean,
    String,
    Number,
    Select,
    StringArray,
    ObjectArray,
    ObjectMap,
    Color,
    Action,
    List,
    Status,
    QrCode,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum FieldDefault {
    Boolean(bool),
    String(String),
    Number(f64),
    StringArray(Vec<String>),
    ObjectArray(Vec<IndexMap<String, FieldDefault>>),
    ObjectMap(IndexMap<String, IndexMap<String, FieldDefault>>),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct NumberConstraints {
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    #[serde(default)]
    pub step: Option<f64>,
}

pub fn parse_spec(path: impl AsRef<Path>) -> Result<ConfigSpec, ParseSpecError> {
    let raw = std::fs::read_to_string(path).map_err(ParseSpecError::Io)?;
    parse_spec_str(&raw).map_err(ParseSpecError::Toml)
}

pub fn parse_spec_str(input: &str) -> Result<ConfigSpec, toml::de::Error> {
    toml::from_str(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_color_field() {
        let spec_str = "
schema_version = 1

[field.border_color]
type = \"color\"
default = \"#5FA8FF\"
";
        let spec = parse_spec_str(spec_str).expect("parse");
        let field = spec.fields.get("border_color").expect("field present");
        assert!(
            matches!(field.kind, FieldKind::Color),
            "expected Color, got {:?}",
            field.kind
        );
        match field.default.as_ref() {
            Some(FieldDefault::String(s)) => assert_eq!(s, "#5FA8FF", "color default"),
            other => panic!("expected String default, got {other:?}"),
        }
    }

    #[test]
    fn parses_color_field_with_alpha() {
        let spec_str = "
schema_version = 1

[field.overlay_color]
type = \"color\"
default = \"#000000FF\"
alpha = true
";
        let spec = parse_spec_str(spec_str).expect("parse");
        let field = spec.fields.get("overlay_color").expect("field present");
        assert!(matches!(field.kind, FieldKind::Color));
        assert_eq!(field.alpha, Some(true), "alpha flag");
    }

    #[test]
    fn parses_action_field() {
        let spec_str = r#"
schema_version = 1

[field.pair_new_device]
type = "action"
label = "Pair New Device"
action = "pair_device"
"#;
        let spec = parse_spec_str(spec_str).expect("parse");
        let field = spec.fields.get("pair_new_device").expect("field present");
        assert!(
            matches!(field.kind, FieldKind::Action),
            "expected Action, got {:?}",
            field.kind
        );
        assert_eq!(
            field.action.as_deref(),
            Some("pair_device"),
            "action reference"
        );
        assert_eq!(field.label.as_deref(), Some("Pair New Device"), "label");
    }

    #[test]
    fn parses_action_field_with_variant() {
        let spec_str = r#"
schema_version = 1

[field.remove_all]
type = "action"
label = "Remove All"
action = "remove_all_devices"
variant = "danger"
"#;
        let spec = parse_spec_str(spec_str).expect("parse");
        let field = spec.fields.get("remove_all").expect("field present");
        assert_eq!(field.variant.as_deref(), Some("danger"), "variant");
    }

    #[test]
    fn parses_list_field() {
        let spec_str = r#"
schema_version = 1

[field.paired_devices]
type = "list"
label = "Paired Devices"
query = "list_devices"
row_label = "{name}"
row_subtitle = "{ieee}"
empty_message = "No devices paired yet."
"#;
        let spec = parse_spec_str(spec_str).expect("parse");
        let field = spec.fields.get("paired_devices").expect("field present");
        assert!(
            matches!(field.kind, FieldKind::List),
            "expected List, got {:?}",
            field.kind
        );
        assert_eq!(field.query.as_deref(), Some("list_devices"), "query");
        assert_eq!(field.row_label.as_deref(), Some("{name}"), "row_label");
        assert_eq!(
            field.row_subtitle.as_deref(),
            Some("{ieee}"),
            "row_subtitle"
        );
        assert_eq!(
            field.empty_message.as_deref(),
            Some("No devices paired yet."),
            "empty_message"
        );
    }

    #[test]
    fn parses_status_field() {
        let spec_str = r#"
schema_version = 1

[field.coordinator_status]
type = "status"
label = "Coordinator"
query = "connection_status"
value_from = "state"

[field.coordinator_status.label_map]
ok = "Connected"
connecting = "Connecting..."
offline = "Offline"

[field.coordinator_status.tone_map]
ok = "success"
connecting = "warning"
offline = "danger"
"#;
        let spec = parse_spec_str(spec_str).expect("parse");
        let field = spec
            .fields
            .get("coordinator_status")
            .expect("field present");
        assert!(matches!(field.kind, FieldKind::Status));
        assert_eq!(field.query.as_deref(), Some("connection_status"));
        assert_eq!(field.value_from.as_deref(), Some("state"));
        let label_map = field.label_map.as_ref().expect("label_map");
        assert_eq!(label_map.get("ok").map(|s| s.as_str()), Some("Connected"));
        assert_eq!(
            label_map.get("offline").map(|s| s.as_str()),
            Some("Offline")
        );
        let tone_map = field.tone_map.as_ref().expect("tone_map");
        assert_eq!(tone_map.get("ok").map(|s| s.as_str()), Some("success"));
    }

    #[test]
    fn parses_qr_code_field() {
        let spec_str = r#"
schema_version = 1

[field.pair_qr]
type = "qr_code"
label = "Scan to pair phone"
query = "pair_url"
value_from = "url"
placeholder = "Waiting for session..."
"#;
        let spec = parse_spec_str(spec_str).expect("parse");
        let field = spec.fields.get("pair_qr").expect("field present");
        assert!(matches!(field.kind, FieldKind::QrCode));
        assert_eq!(field.query.as_deref(), Some("pair_url"));
        assert_eq!(field.value_from.as_deref(), Some("url"));
        assert_eq!(field.placeholder.as_deref(), Some("Waiting for session..."));
    }
}
