mod store;

#[cfg(test)]
mod tests;

use crate::paths;
use crate::paths::is_safe_path_component;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginConfigs {
    #[serde(flatten)]
    pub configs: HashMap<String, serde_json::Value>,
}

pub struct PluginConfigManager {
    config_path: PathBuf,
}

impl PluginConfigManager {
    pub fn new() -> Result<Self> {
        let config_path = paths::plugin_configs_path()?;
        Ok(Self { config_path })
    }

    fn plugin_config_path(plugin_id: &str) -> Result<PathBuf> {
        if !is_safe_path_component(plugin_id) {
            anyhow::bail!("Invalid plugin ID: {}", plugin_id);
        }
        paths::plugins_dir().map(|path| path.join(plugin_id).join("config.json"))
    }

    pub fn load_configs(&self) -> Result<PluginConfigs> {
        store::load_configs(&self.config_path)
    }

    pub fn save_configs(&self, configs: &PluginConfigs) -> Result<()> {
        store::save_configs(&self.config_path, configs)
    }

    pub fn get_config(&self, plugin_id: &str) -> Result<Option<serde_json::Value>> {
        let plugin_path = Self::plugin_config_path(plugin_id)?;
        if plugin_path.exists() {
            return store::load_plugin_config(&plugin_path).map(Some);
        }
        self.restore_from_backup(plugin_id)
    }

    pub fn set_config(&self, plugin_id: &str, config: serde_json::Value) -> Result<()> {
        let plugin_path = Self::plugin_config_path(plugin_id)?;
        store::write_plugin_config(&plugin_path, &config)?;
        let mut configs = self.load_configs()?;
        configs.configs.insert(plugin_id.to_string(), config);
        self.save_configs(&configs)
    }

    fn restore_from_backup(&self, plugin_id: &str) -> Result<Option<serde_json::Value>> {
        let configs = self.load_configs()?;
        let Some(config) = configs.configs.get(plugin_id).cloned() else {
            return Ok(None);
        };
        let plugin_path = Self::plugin_config_path(plugin_id)?;
        log::info!("Restoring config for plugin from backup: {}", plugin_id);
        store::write_plugin_config(&plugin_path, &config)?;
        log::info!("Config restored for plugin: {}", plugin_id);
        Ok(Some(config))
    }
}
