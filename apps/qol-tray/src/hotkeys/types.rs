use crate::plugins::PluginUid;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HotkeyConfig {
    #[serde(default)]
    pub hotkeys: Vec<HotkeyBinding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyBinding {
    pub id: String,
    pub key: String,
    #[serde(alias = "plugin_id")]
    pub plugin_uid: PluginUid,
    pub action: String,
    #[serde(default)]
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyAction {
    pub plugin_uid: PluginUid,
    pub action: String,
}
