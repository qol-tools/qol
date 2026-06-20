use serde::Deserialize;

use crate::paths::PLUGIN_ID;
use crate::placement::Corner;

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct CliSessionsConfig {
    pub corner: Option<String>,
    pub service_commands: Vec<String>,
}

impl CliSessionsConfig {
    pub fn corner(&self) -> Corner {
        Corner::parse(self.corner.as_deref().unwrap_or_default())
    }
}

pub fn load() -> CliSessionsConfig {
    qol_config::load_plugin_config_from_env(PLUGIN_ID)
}
