use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ShortcutsConfig {
    #[serde(default)]
    pub shortcuts: Vec<Shortcut>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Shortcut {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub export_to_launcher: bool,
    pub action: ShortcutAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ShortcutAction {
    #[serde(rename = "open_url")]
    OpenUrl {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        browser_override: Option<AppRef>,
    },
    #[serde(rename = "launch_app")]
    LaunchApp { app: AppRef },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AppRef {
    #[serde(rename = "bundle_id")]
    BundleId { id: String },
    #[serde(rename = "path")]
    Path { path: String },
    #[serde(rename = "name")]
    Name { name: String },
}

fn default_true() -> bool {
    true
}
