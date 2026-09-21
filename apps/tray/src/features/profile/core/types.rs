use crate::plugins::PluginUid;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const CURRENT_PROFILE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileManifest {
    #[serde(default = "default_profile_version")]
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PluginLockEntry {
    pub uid: PluginUid,
    pub id: String,
    pub repo_url: String,
    pub version: String,
    pub platforms: Option<Vec<String>>,
}

impl<'de> Deserialize<'de> for PluginLockEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            uid: Option<PluginUid>,
            id: String,
            repo_url: String,
            #[serde(default)]
            version: String,
            #[serde(default)]
            platforms: Option<Vec<String>>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let uid = raw.uid.unwrap_or_else(|| PluginUid::new(raw.id.as_str()));
        Ok(PluginLockEntry {
            uid,
            id: raw.id,
            repo_url: raw.repo_url,
            version: raw.version,
            platforms: raw.platforms,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginsLock {
    #[serde(default = "default_profile_version")]
    pub version: u32,
    #[serde(default)]
    pub plugins: Vec<PluginLockEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileExportBundle {
    #[serde(default = "default_profile_version")]
    pub version: u32,
    pub exported_at: String,
    #[serde(default)]
    pub hotkeys: Vec<Value>,
    #[serde(default)]
    pub shortcuts: Vec<Value>,
    pub task_runner: Value,
    #[serde(default)]
    pub plugin_configs: HashMap<String, Value>,
    #[serde(default)]
    pub plugins: Vec<PluginLockEntry>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProfileImportBundle {
    #[serde(default, deserialize_with = "deserialize_hotkeys_field")]
    pub hotkeys: Option<Vec<Value>>,
    #[serde(default, deserialize_with = "deserialize_shortcuts_field")]
    pub shortcuts: Option<Vec<Value>>,
    #[serde(default)]
    pub task_runner: Option<Value>,
    #[serde(default)]
    pub plugin_configs: Option<HashMap<String, Value>>,
    #[serde(default)]
    pub plugins: Vec<PluginLockEntry>,
    #[serde(default)]
    pub installed_plugins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ImportPluginResult {
    pub id: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApplyProfileResult {
    pub success: bool,
    pub plugins: Vec<ImportPluginResult>,
}

fn default_profile_version() -> u32 {
    CURRENT_PROFILE_VERSION
}

fn deserialize_hotkeys_field<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Vec<Value>>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_wrapped_array_field(deserializer, "hotkeys")
}

fn deserialize_shortcuts_field<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Vec<Value>>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_wrapped_array_field(deserializer, "shortcuts")
}

fn deserialize_wrapped_array_field<'de, D>(
    deserializer: D,
    field_name: &str,
) -> std::result::Result<Option<Vec<Value>>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    normalize_wrapped_array_field(value, field_name).map_err(serde::de::Error::custom)
}

fn normalize_wrapped_array_field(
    value: Option<Value>,
    field_name: &str,
) -> std::result::Result<Option<Vec<Value>>, String> {
    if value.is_none() {
        return Ok(None);
    }
    let value = value.unwrap();
    if value.is_null() {
        return Ok(None);
    }
    if let Value::Array(items) = value {
        return Ok(Some(items));
    }
    if let Value::Object(mut object) = value {
        let Some(items) = object.remove(field_name) else {
            return Err(format!("profile {field_name} must be an array"));
        };
        if let Value::Array(items) = items {
            return Ok(Some(items));
        }
    }
    Err(format!("profile {field_name} must be an array"))
}
