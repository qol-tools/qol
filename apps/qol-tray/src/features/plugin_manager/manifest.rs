use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginManifest {
    pub plugin: PluginInfo,
    pub menu: MenuConfig,
    #[serde(default)]
    pub daemon: Option<DaemonConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginInfo {
    pub name: String,
    pub description: String,
    pub version: String,
    #[serde(default)]
    pub author: Option<String>,
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
    Custom,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DaemonConfig {
    pub enabled: bool,
    pub command: String,
    #[serde(default)]
    pub restart_on_crash: bool,
}
