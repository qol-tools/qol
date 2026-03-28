mod store;

#[cfg(test)]
mod tests;

use crate::paths;
use crate::paths::is_safe_path_component;
use crate::plugins::paths as plugin_paths;
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
    configs_dir: PathBuf,
}

impl PluginConfigManager {
    pub fn new() -> Result<Self> {
        let configs_dir = paths::profile_plugin_configs_dir()?;
        Ok(Self { configs_dir })
    }

    fn plugin_config_path(plugin_id: &str) -> Result<PathBuf> {
        if !is_safe_path_component(plugin_id) {
            anyhow::bail!("Invalid plugin ID: {}", plugin_id);
        }
        let plugins_dir = paths::plugins_dir()?;
        Ok(plugin_paths::config_path(&plugins_dir.join(plugin_id)))
    }

    pub fn load_configs(&self) -> Result<PluginConfigs> {
        let configs = store::load_configs(&self.configs_dir)?;
        Ok(PluginConfigs { configs })
    }

    pub fn save_configs(&self, configs: &PluginConfigs) -> Result<()> {
        store::save_configs(&self.configs_dir, &configs.configs)
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
        store::write_profile_plugin_config(&self.configs_dir, plugin_id, &config)
    }

    fn restore_from_backup(&self, plugin_id: &str) -> Result<Option<serde_json::Value>> {
        let config = match store::load_profile_plugin_config(&self.configs_dir, plugin_id)? {
            Some(config) => config,
            None => return Ok(None),
        };
        let plugin_path = Self::plugin_config_path(plugin_id)?;
        log::info!("Restoring config for plugin from backup: {}", plugin_id);
        store::write_plugin_config(&plugin_path, &config)?;
        log::info!("Config restored for plugin: {}", plugin_id);
        Ok(Some(config))
    }
}
