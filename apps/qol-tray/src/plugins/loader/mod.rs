mod manifest_loader;
mod scan;

#[cfg(test)]
mod tests;

use super::Plugin;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub struct PluginLoader;

impl PluginLoader {
    pub fn default_plugin_dir() -> Result<PathBuf> {
        crate::paths::plugins_dir()
    }

    pub fn ensure_plugin_dir() -> Result<PathBuf> {
        let dir = Self::default_plugin_dir()?;
        if dir.exists() {
            return Ok(dir);
        }
        std::fs::create_dir_all(&dir).context("Failed to create plugins directory")?;
        log::info!("Created plugins directory: {:?}", dir);
        Ok(dir)
    }

    #[cfg(test)]
    pub fn load_from_dir(dir: &Path) -> Result<Vec<Plugin>> {
        let items = scan::discover_plugin_items(dir)?;
        scan::load_items(&items)
    }

    pub fn load_resolved(resolved: &[super::resolver::ResolvedPlugin]) -> Result<Vec<Plugin>> {
        Ok(resolved
            .iter()
            .filter_map(|r| match manifest_loader::load_resolved_plugin(r) {
                Ok(plugin) => Some(plugin),
                Err(e) => {
                    log::warn!("Skipping plugin {}: {}", r.id, e);
                    None
                }
            })
            .collect())
    }

    #[cfg(test)]
    pub fn load_plugin(path: &Path) -> Result<Plugin> {
        let id = plugin_id_from_path(path)?;
        manifest_loader::load_plugin_with_id(&id, path)
    }

    pub fn load_plugin_with_id(id: &str, path: &Path) -> Result<Plugin> {
        manifest_loader::load_plugin_with_id(id, path)
    }
}

#[cfg(test)]
fn plugin_id_from_path(path: &Path) -> Result<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .context("Invalid plugin directory name")
}
