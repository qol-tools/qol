use crate::plugins::PluginId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginManifest {
    #[serde(default = "super::default_manifest_version")]
    pub manifest_version: u32,
    pub plugin: PluginInfo,
    pub menu: MenuConfig,
    #[serde(default)]
    pub daemon: Option<DaemonConfig>,
    #[serde(default)]
    pub dependencies: Option<Dependencies>,
    #[serde(default)]
    pub runtime: Option<RuntimeConfig>,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub build: BuildInfo,
    #[serde(default)]
    pub traits: Option<serde_json::Value>,
    #[serde(default)]
    pub config: ConfigDeclarations,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ConfigDeclarations {
    #[serde(default)]
    pub default_scope: Option<ConfigScope>,
    #[serde(default)]
    pub scope: HashMap<String, ConfigScope>,
}

impl ConfigDeclarations {
    pub fn scope_for(&self, field: &str) -> ConfigScope {
        self.scope
            .get(field)
            .copied()
            .or(self.default_scope)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum ConfigScope {
    #[default]
    #[serde(alias = "any")]
    Core,
    Os,
    Device,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct BuildInfo {
    #[serde(default)]
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RuntimeConfig {
    pub command: String,
    #[serde(default)]
    pub actions: Option<HashMap<String, Vec<String>>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Dependencies {
    #[serde(default)]
    pub binaries: Vec<BinaryDependency>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BinaryDependency {
    pub name: String,
    pub repo: String,
    pub pattern: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginInfo {
    #[serde(default)]
    pub id: Option<PluginId>,
    pub name: String,
    pub description: String,
    pub version: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub platforms: Option<Vec<String>>,
}

impl PluginInfo {
    pub fn supports_current_platform(&self) -> bool {
        super::supports_current_platform(&self.platforms)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MenuConfig {
    pub label: String,
    #[serde(default)]
    pub icon: Option<String>,
    pub items: Vec<MenuItem>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MenuItem {
    Action {
        id: String,
        label: String,
        action: ActionType,
        #[serde(default)]
        config_key: Option<String>,
    },
    Checkbox {
        id: String,
        label: String,
        #[serde(default)]
        checked: bool,
        action: ActionType,
        #[serde(default)]
        config_key: Option<String>,
    },
    Separator,
    Submenu {
        id: String,
        label: String,
        items: Vec<MenuItem>,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ActionType {
    Run,
    Settings,
    #[serde(rename = "toggle-config")]
    ToggleConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DaemonConfig {
    pub enabled: bool,
    pub command: String,
    #[serde(default)]
    pub socket: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Capabilities {
    #[serde(default)]
    pub serial: bool,
    #[serde(flatten)]
    pub extras: HashMap<String, toml::Value>,
}
