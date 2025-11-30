use super::{Plugin, PluginManifest};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub struct PluginLoader;

impl PluginLoader {
    pub fn default_plugin_dir() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .context("Could not determine config directory")?;
        Ok(config_dir.join("qol-tray").join("plugins"))
    }

    pub fn ensure_plugin_dir() -> Result<PathBuf> {
        let dir = Self::default_plugin_dir()?;
        if !dir.exists() {
            fs::create_dir_all(&dir)
                .context("Failed to create plugins directory")?;
            log::info!("Created plugins directory: {:?}", dir);
        }
        Ok(dir)
    }

    pub fn load_all() -> Result<Vec<Plugin>> {
        let plugin_dir = Self::ensure_plugin_dir()?;
        Self::load_from_dir(&plugin_dir)
    }

    pub fn load_from_dir(dir: &Path) -> Result<Vec<Plugin>> {
        let mut plugins = Vec::new();

        if !dir.exists() {
            log::warn!("Plugin directory does not exist: {:?}", dir);
            return Ok(plugins);
        }

        let entries = fs::read_dir(dir)
            .context("Failed to read plugins directory")?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if !path.is_dir() {
                continue;
            }

            match Self::load_plugin(&path) {
                Ok(plugin) => {
                    log::info!("Loaded plugin: {} ({})", plugin.manifest.plugin.name, plugin.id);
                    plugins.push(plugin);
                }
                Err(e) => {
                    log::warn!("Failed to load plugin from {:?}: {}", path, e);
                }
            }
        }

        log::info!("Loaded {} plugin(s)", plugins.len());
        Ok(plugins)
    }

    pub fn load_plugin(path: &Path) -> Result<Plugin> {
        let manifest_path = path.join("plugin.toml");

        if !manifest_path.exists() {
            anyhow::bail!("No plugin.toml found in {:?}", path);
        }

        let manifest_content = fs::read_to_string(&manifest_path)
            .context("Failed to read plugin.toml")?;

        let manifest: PluginManifest = toml::from_str(&manifest_content)
            .context("Failed to parse plugin.toml")?;

        let id = path.file_name()
            .and_then(|n| n.to_str())
            .context("Invalid plugin directory name")?
            .to_string();

        Ok(Plugin::new(id, manifest, path.to_path_buf()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_plugin_dir() {
        let dir = PluginLoader::default_plugin_dir().unwrap();
        assert!(dir.ends_with("qol-tray/plugins"));
    }
}
