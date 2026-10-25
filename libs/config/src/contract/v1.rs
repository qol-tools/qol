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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages: Option<PagesSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devices: Option<DevicesSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowSpec>,
}

impl ConfigSpecV1 {
    pub fn field(&self, id: &str) -> Option<&FieldSpec> {
        self.fields.get(id)
    }

    pub fn section(&self, id: &str) -> Option<&SectionSpec> {
        self.sections.get(id)
    }

    fn add_window_fields(&mut self) -> Result<(), toml::de::Error> {
        let Some(window) = &self.window else {
            return Ok(());
        };
        let shared = window_contract(window.title_bar);
        let taken = shared
            .sections
            .keys()
            .find(|id| self.sections.contains_key(*id))
            .or_else(|| {
                shared
                    .fields
                    .keys()
                    .find(|id| self.fields.contains_key(*id))
            });
        if let Some(id) = taken {
            return Err(serde::de::Error::custom(format!(
                "[window] adds `{id}`, which this contract already declares"
            )));
        }
        self.sections.extend(shared.sections);
        self.fields.extend(shared.fields);
        Ok(())
    }
}

pub const TITLE_BAR_KEY: &str = "title_bar";

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WindowSpec {
    pub title_bar: bool,
}

fn window_contract(title_bar: bool) -> ConfigSpecV1 {
    toml::from_str(&format!(
        r#"schema_version = 1

[section.window]
label = "Window"
description = "How this plugin's window looks."

[field.{TITLE_BAR_KEY}]
type = "boolean"
config_key = "{TITLE_BAR_KEY}"
label = "System title bar"
description = "Show the system's title bar and its buttons, and resize from the system's frame. Takes effect the next time the window opens."
section = "window"
default = {title_bar}
"#
    ))
    .expect("the shared window contract parses")
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PagesSpec {
    pub query: String,
    pub action: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DevicesSpec {
    pub query: String,
    pub kinds: IndexMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct SectionSpec {
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub actions: Vec<String>,
    #[serde(default)]
    pub dev_only: bool,
    #[serde(default)]
    pub platforms: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_kind: Option<String>,
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
    pub option_pictures: IndexMap<String, String>,
    #[serde(default)]
    pub card_description: Option<String>,
    #[serde(default)]
    pub item_label: Option<String>,
    #[serde(default)]
    pub lookup_label: Option<String>,
    #[serde(default)]
    pub add_label: Option<String>,
    #[serde(default)]
    pub reorder: Option<bool>,
    #[serde(default)]
    pub lists: IndexMap<String, NestedListSpec>,
    #[serde(default)]
    pub key_label: Option<String>,
    #[serde(default)]
    pub item: Option<ItemSpec>,
    #[serde(default)]
    pub entry_fields: IndexMap<String, FieldKind>,
    #[serde(default)]
    pub show_when: Option<ShowWhenSpec>,
    #[serde(default)]
    pub platforms: Option<Vec<String>>,
    #[serde(default)]
    pub alpha: Option<bool>,
    #[serde(default)]
    pub action: Option<String>,
    #[serde(default)]
    pub active_action: Option<String>,
    #[serde(default)]
    pub active_query: Option<String>,
    #[serde(default)]
    pub active_value_from: Option<String>,
    #[serde(default)]
    pub active_label: Option<String>,
    #[serde(default)]
    pub level_query: Option<String>,
    #[serde(default)]
    pub level_value_from: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub row_label: Option<String>,
    #[serde(default)]
    pub row_subtitle: Option<String>,
    #[serde(default)]
    pub row_action: Option<RowActionSpec>,
    #[serde(default)]
    pub row_actions: Vec<RowActionSpec>,
    #[serde(default)]
    pub row_slider: Option<RowSliderSpec>,
    #[serde(default)]
    pub lead: Option<ListLeadSpec>,
    #[serde(default)]
    pub items_label: Option<String>,
    #[serde(default)]
    pub item_card_field: Option<String>,
    #[serde(default)]
    pub empty_message: Option<String>,
    #[serde(default)]
    pub value_from: Option<String>,
    #[serde(default)]
    pub stream: Option<String>,
    #[serde(default)]
    pub label_map: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub tone_map: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub hide_values: Option<Vec<String>>,
    #[serde(default)]
    pub align: Option<FieldAlign>,
    #[serde(flatten)]
    pub number: NumberConstraints,
}

impl FieldSpec {
    pub fn has_stored_value(&self) -> bool {
        self.kind.has_stored_value()
            && !(self.kind == FieldKind::Number && self.active_query.is_some())
    }

    pub fn requires_declared_default(&self) -> bool {
        self.kind.has_stored_value()
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FieldAlign {
    Left,
    Center,
    Right,
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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct NestedListSpec {
    #[serde(default)]
    pub item_label: Option<String>,
    #[serde(default)]
    pub card_description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RowActionSpec {
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub open: Option<String>,
    #[serde(default)]
    pub input: Option<IndexMap<String, String>>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub when: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ListLeadSpec {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub row_label: Option<String>,
    #[serde(default)]
    pub row_subtitle: Option<String>,
    #[serde(default)]
    pub actions: Vec<RowActionSpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RowSliderSpec {
    pub value_from: String,
    #[serde(default = "default_slider_min")]
    pub min: f64,
    #[serde(default = "default_slider_max")]
    pub max: f64,
    #[serde(default = "default_slider_step")]
    pub step: f64,
    pub action: String,
    #[serde(default)]
    pub input: Option<IndexMap<String, String>>,
}

fn default_slider_min() -> f64 {
    0.0
}

fn default_slider_max() -> f64 {
    100.0
}

fn default_slider_step() -> f64 {
    1.0
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
    Gamepad,
    DisplayLayout,
    Placement,
}

impl FieldKind {
    pub fn has_stored_value(self) -> bool {
        !matches!(
            self,
            Self::Action
                | Self::List
                | Self::Status
                | Self::QrCode
                | Self::Gamepad
                | Self::DisplayLayout
        )
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::String => "string",
            Self::Number => "number",
            Self::Select => "select",
            Self::StringArray => "string_array",
            Self::ObjectArray => "object_array",
            Self::ObjectMap => "object_map",
            Self::Color => "color",
            Self::Action => "action",
            Self::List => "list",
            Self::Status => "status",
            Self::QrCode => "qr_code",
            Self::Gamepad => "gamepad",
            Self::DisplayLayout => "display_layout",
            Self::Placement => "placement",
        }
    }
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

impl NumberConstraints {
    pub const DEFAULT_MIN: f64 = 0.0;
    pub const DEFAULT_MAX: f64 = i32::MAX as f64;

    pub fn lowest(&self) -> f64 {
        self.min.unwrap_or(Self::DEFAULT_MIN)
    }

    pub fn highest(&self) -> f64 {
        self.max.unwrap_or(Self::DEFAULT_MAX)
    }

    pub fn clamp(&self, value: f64) -> f64 {
        value.max(self.lowest()).min(self.highest())
    }
}

pub fn parse_spec(path: impl AsRef<Path>) -> Result<ConfigSpec, ParseSpecError> {
    let raw = std::fs::read_to_string(path).map_err(ParseSpecError::Io)?;
    parse_spec_str(&raw).map_err(ParseSpecError::Toml)
}

pub fn parse_spec_str(input: &str) -> Result<ConfigSpec, toml::de::Error> {
    let mut spec: ConfigSpec = toml::from_str(input)?;
    spec.add_window_fields()?;
    Ok(spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_platform::Os;

    #[test]
    fn number_bounds_default_to_zero_and_the_largest_i32() {
        let cases = [
            (None, None, -5.0, 0.0),
            (None, None, 3e12, f64::from(i32::MAX)),
            (Some(-10.0), None, -20.0, -10.0),
            (None, Some(100.0), 150.0, 100.0),
            (Some(0.0), Some(100.0), 42.0, 42.0),
        ];
        for (min, max, value, expected) in cases {
            let bounds = NumberConstraints {
                min,
                max,
                step: None,
            };
            assert_eq!(bounds.clamp(value), expected, "{min:?}..{max:?} {value}");
        }
    }

    #[test]
    fn a_window_table_adds_the_shared_title_bar_switch_with_its_default() {
        for title_bar in [true, false] {
            let spec = parse_spec_str(&format!(
                "schema_version = 1\n\n[window]\ntitle_bar = {title_bar}\n"
            ))
            .unwrap();
            let field = spec.field(TITLE_BAR_KEY).expect("title bar field");
            assert_eq!(field.kind, FieldKind::Boolean);
            assert_eq!(field.section.as_deref(), Some("window"));
            assert_eq!(field.default, Some(FieldDefault::Boolean(title_bar)));
            assert!(spec.section("window").is_some());
        }
        let plain = parse_spec_str("schema_version = 1\n").unwrap();
        assert!(plain.field(TITLE_BAR_KEY).is_none());
    }

    #[test]
    fn a_window_table_refuses_a_contract_that_already_declares_its_switch() {
        for own in [
            "[field.title_bar]\ntype = \"boolean\"\nconfig_key = \"title_bar\"\nlabel = \"Mine\"\ndescription = \"Mine.\"\ndefault = true\n",
            "[section.window]\nlabel = \"Mine\"\ndescription = \"Mine.\"\n",
        ] {
            let error = parse_spec_str(&format!(
                "schema_version = 1\n\n[window]\ntitle_bar = true\n\n{own}"
            ))
            .unwrap_err();
            assert!(error.to_string().contains("already declares"), "{error}");
        }
    }

    #[test]
    fn field_order_follows_declaration_not_alphabet() {
        let spec_str = "
schema_version = 1

[field.zeta]
type = \"boolean\"
default = true

[field.alpha]
type = \"number\"
default = 1
";
        let spec = parse_spec_str(spec_str).expect("valid spec");
        let order: Vec<&str> = spec.fields.keys().map(String::as_str).collect();
        assert_eq!(order, ["zeta", "alpha"]);
    }

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
    fn parses_action_field_with_runtime_active_state() {
        let spec_str = r#"
schema_version = 1

[field.search]
type = "action"
label = "Start search"
action = "start_search"
active_action = "stop_search"
active_query = "search_status"
active_value_from = "searching"
active_label = "Stop search"
"#;
        let spec = parse_spec_str(spec_str).expect("parse");
        let field = spec.fields.get("search").expect("field present");
        assert_eq!(field.action.as_deref(), Some("start_search"));
        assert_eq!(field.active_action.as_deref(), Some("stop_search"));
        assert_eq!(field.active_query.as_deref(), Some("search_status"));
        assert_eq!(field.active_value_from.as_deref(), Some("searching"));
        assert_eq!(field.active_label.as_deref(), Some("Stop search"));
    }

    #[test]
    fn parses_level_slider_number_field() {
        let spec_str = r#"
schema_version = 1

[field.volume]
type = "number"
default = 0
variant = "level_slider"
level_query = "levels"
level_value_from = "output"
"#;
        let spec = parse_spec_str(spec_str).expect("parse");
        let field = spec.fields.get("volume").expect("field present");
        assert_eq!(field.variant.as_deref(), Some("level_slider"));
        assert_eq!(field.level_query.as_deref(), Some("levels"));
        assert_eq!(field.level_value_from.as_deref(), Some("output"));
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
    fn parses_list_row_action_with_when_gate() {
        let spec_str = r#"
schema_version = 1

[field.pads]
type = "list"
query = "list_controllers"
row_label = "{name}"

[field.pads.row_action]
action = "apply_fixes"
label = "Fix"
description = "Applies the fix."
when = "fixable"
"#;
        let spec = parse_spec_str(spec_str).expect("parse");
        let field = spec.fields.get("pads").expect("field present");
        let row_action = field.row_action.as_ref().expect("row_action present");
        assert_eq!(row_action.action, "apply_fixes", "action");
        assert_eq!(row_action.label.as_deref(), Some("Fix"), "label");
        assert_eq!(
            row_action.description.as_deref(),
            Some("Applies the fix."),
            "description"
        );
        assert_eq!(row_action.when.as_deref(), Some("fixable"), "when");
    }

    #[test]
    fn parses_list_row_slider_with_default_range() {
        let spec_str = r#"
schema_version = 1

[field.volumes]
type = "list"
query = "list_volumes"
row_label = "{name}"

[field.volumes.row_slider]
value_from = "volume"
action = "set_volume"
input = { id = "{id}", value = "{value}" }
"#;
        let spec = parse_spec_str(spec_str).expect("parse");
        let field = spec.fields.get("volumes").expect("field present");
        let slider = field.row_slider.as_ref().expect("row_slider present");
        assert_eq!(slider.value_from, "volume", "value_from");
        assert_eq!(slider.min, 0.0, "default min");
        assert_eq!(slider.max, 100.0, "default max");
        assert_eq!(slider.step, 1.0, "default step");
        assert_eq!(slider.action, "set_volume", "action");
        assert_eq!(
            slider.input.as_ref().unwrap()["value"],
            "{value}",
            "input template"
        );
    }

    #[test]
    fn parses_list_row_slider_with_explicit_range() {
        let spec_str = r#"
schema_version = 1

[field.brightness]
type = "list"
query = "list_displays"

[field.brightness.row_slider]
value_from = "level"
min = 5
max = 250
step = 5
action = "set_brightness"
"#;
        let spec = parse_spec_str(spec_str).expect("parse");
        let field = spec.fields.get("brightness").expect("field present");
        let slider = field.row_slider.as_ref().expect("row_slider present");
        assert_eq!(slider.value_from, "level");
        assert_eq!(slider.min, 5.0);
        assert_eq!(slider.max, 250.0);
        assert_eq!(slider.step, 5.0);
        assert_eq!(slider.action, "set_brightness");
        assert!(slider.input.is_none());
    }

    #[test]
    fn rejects_list_row_slider_missing_required_keys() {
        let missing_value_from = parse_spec_str(
            r#"
schema_version = 1

[field.volumes]
type = "list"
query = "list_volumes"

[field.volumes.row_slider]
action = "set_volume"
"#,
        );
        assert!(
            missing_value_from.is_err(),
            "value_from is required: {missing_value_from:?}"
        );

        let missing_action = parse_spec_str(
            r#"
schema_version = 1

[field.volumes]
type = "list"
query = "list_volumes"

[field.volumes.row_slider]
value_from = "volume"
"#,
        );
        assert!(
            missing_action.is_err(),
            "action is required: {missing_action:?}"
        );
    }

    #[test]
    fn parses_list_with_state_driven_row_actions() {
        let spec = parse_spec_str(
            r#"
schema_version = 1

[field.devices]
type = "list"
query = "devices"

[[field.devices.row_actions]]
action = "pair_device"
label = "Pair"
when = "can_pair"
input = { address = "{address}" }

[[field.devices.row_actions]]
action = "connect_device"
label = "Connect"
when = "can_connect"
input = { address = "{address}" }
"#,
        )
        .expect("parse config");
        let actions = &spec
            .fields
            .get("devices")
            .expect("devices field")
            .row_actions;
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action, "pair_device");
        assert_eq!(actions[0].input.as_ref().unwrap()["address"], "{address}");
        assert_eq!(actions[1].when.as_deref(), Some("can_connect"));
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

    #[test]
    fn parses_gamepad_field() {
        let spec_str = r#"
schema_version = 1

[field.input_test]
type = "gamepad"
label = "Input Test"
description = "Press a button to begin."
"#;
        let spec = parse_spec_str(spec_str).expect("parse");
        let field = spec.fields.get("input_test").expect("field present");
        assert_eq!(field.kind, FieldKind::Gamepad);
        assert!(!field.kind.has_stored_value());
        assert_eq!(field.label.as_deref(), Some("Input Test"));
    }

    #[test]
    fn parses_display_layout_field() {
        let spec_str = r#"
schema_version = 1

[field.arrangement]
type = "display_layout"
label = "Arrangement"
query = "layout"
active_query = "modes"
action = "arrange"
active_action = "set_mode"
"#;
        let spec = parse_spec_str(spec_str).expect("parse");
        let field = spec.fields.get("arrangement").expect("field present");
        assert_eq!(field.kind, FieldKind::DisplayLayout);
        assert!(!field.kind.has_stored_value());
        assert_eq!(field.kind.name(), "display_layout");
        assert_eq!(field.query.as_deref(), Some("layout"));
        assert_eq!(field.active_query.as_deref(), Some("modes"));
        assert_eq!(field.action.as_deref(), Some("arrange"));
        assert_eq!(field.active_action.as_deref(), Some("set_mode"));
    }

    #[test]
    fn parses_platforms_on_fields_and_sections() {
        let spec = parse_spec_str(
            r#"
schema_version = 1

[section.host]
label = "System"
platforms = ["linux"]

[field.frame]
type = "boolean"
default = true
platforms = ["linux", "windows"]

[field.everywhere]
type = "boolean"
default = true
"#,
        )
        .expect("parse");
        assert_eq!(
            spec.section("host").unwrap().platforms,
            Some(vec![Os::Linux.name().to_string()])
        );
        assert_eq!(
            spec.field("frame").unwrap().platforms,
            Some(vec![
                Os::Linux.name().to_string(),
                Os::Windows.name().to_string()
            ])
        );
        assert_eq!(spec.field("everywhere").unwrap().platforms, None);
    }

    #[test]
    fn parses_pages_table() {
        let spec = parse_spec_str(
            "
schema_version = 1

[pages]
query = \"pages\"
action = \"press\"
",
        )
        .expect("valid spec");
        assert_eq!(
            spec.pages,
            Some(PagesSpec {
                query: "pages".into(),
                action: "press".into(),
            })
        );
        assert!(spec.fields.is_empty());
    }

    #[test]
    fn parses_a_devices_table_and_the_kind_each_section_belongs_to() {
        let spec = parse_spec_str(
            "
schema_version = 1

[devices]
query = \"devices\"
kinds = { mouse = \"Mouse\", keyboard = \"Keyboards\" }

[section.pointer]
label = \"Pointer\"
device_kind = \"mouse\"
",
        )
        .expect("valid spec");
        let devices = spec.devices.expect("a devices table");
        assert_eq!(devices.query, "devices");
        assert_eq!(
            devices.kinds.into_iter().collect::<Vec<_>>(),
            [
                ("mouse".to_string(), "Mouse".to_string()),
                ("keyboard".to_string(), "Keyboards".to_string())
            ]
        );
        assert_eq!(
            spec.sections["pointer"].device_kind.as_deref(),
            Some("mouse")
        );
    }

    #[test]
    fn rejects_unknown_pages_keys() {
        let error = parse_spec_str(
            "
schema_version = 1

[pages]
query = \"pages\"
action = \"press\"
poll = 5
",
        )
        .expect_err("unknown key");
        assert!(error.to_string().contains("poll"), "{error}");
    }
}
