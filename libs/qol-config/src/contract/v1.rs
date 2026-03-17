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
