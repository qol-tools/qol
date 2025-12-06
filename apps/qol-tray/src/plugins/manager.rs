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

    #[cfg(feature = "dev")]
    pub fn reload_plugins(&mut self) -> Result<()> {
        log::info!("Reloading all plugins...");
        for plugin in self.plugins.values_mut() {
            if let Err(e) = plugin.stop_daemon() {
                log::error!("Failed to stop daemon for plugin {}: {}", plugin.id, e);
            }
        }
        self.plugins.clear();
        self.load_plugins()
    }

    pub fn plugins(&self) -> impl Iterator<Item = &Plugin> {
        self.plugins.values()
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}
