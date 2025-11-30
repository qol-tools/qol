use super::{Plugin, PluginLoader};
use anyhow::Result;
use std::collections::HashMap;

pub struct PluginManager {
    plugins: HashMap<String, Plugin>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
        }
    }

    pub fn load_plugins(&mut self) -> Result<()> {
        let plugins = PluginLoader::load_all()?;

        for mut plugin in plugins {
            if let Err(e) = plugin.start_daemon() {
                log::error!("Failed to start daemon for plugin {}: {}", plugin.id, e);
            }

            self.plugins.insert(plugin.id.clone(), plugin);
        }

        Ok(())
    }

    pub fn reload_plugins(&mut self) -> Result<()> {
        log::info!("Reloading plugins...");

        // Stop all daemons
        for plugin in self.plugins.values_mut() {
            let _ = plugin.stop_daemon();
        }

        // Clear existing plugins
        self.plugins.clear();

        // Load fresh
        self.load_plugins()
    }

    pub fn get(&self, id: &str) -> Option<&Plugin> {
        self.plugins.get(id)
    }

    pub fn plugins(&self) -> impl Iterator<Item = &Plugin> {
        self.plugins.values()
    }

    pub fn execute_plugin(&self, id: &str) -> Result<()> {
        let plugin = self.get(id)
            .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", id))?;
        plugin.execute()
    }

    pub fn update_plugin_config(&self, id: &str, key: &str, value: serde_json::Value) -> Result<()> {
        let plugin = self.get(id)
            .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", id))?;
        plugin.update_config(key, value)
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}
