use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    #[serde(default)]
    pub export_plugin_actions_to_launcher: bool,
}

pub fn load() -> AppSettings {
    crate::paths::app_settings_path()
        .ok()
        .and_then(|p| crate::file_io::load_json_or_default(&p).ok())
        .unwrap_or_default()
}
