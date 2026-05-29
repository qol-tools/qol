use super::{PluginMetadata, CACHE_FORMAT_VERSION};
use crate::paths;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn cache_path() -> Option<PathBuf> {
    paths::plugin_cache_path().ok()
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct PluginCache {
    #[serde(default)]
    pub format_version: u32,
    pub timestamp: u64,
    pub plugins: Vec<CachedPlugin>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct CachedPlugin {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub repo_url: String,
    #[serde(default)]
    pub platforms: Option<Vec<String>>,
}

impl From<PluginMetadata> for CachedPlugin {
    fn from(metadata: PluginMetadata) -> Self {
        Self {
            id: metadata.id,
            name: metadata.name,
            description: metadata.description,
            version: metadata.version,
            repo_url: metadata.repo_url,
            platforms: metadata.platforms,
        }
    }
}

impl From<CachedPlugin> for PluginMetadata {
    fn from(cached: CachedPlugin) -> Self {
        Self {
            id: cached.id,
            name: cached.name,
            description: cached.description,
            version: cached.version,
            repo_url: cached.repo_url,
            platforms: cached.platforms,
        }
    }
}

pub(crate) fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(crate) fn read_cache() -> Option<PluginCache> {
    let path = cache_path()?;
    let content = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&content).ok()
}

pub(crate) fn write_cache(plugins: &[PluginMetadata]) -> Result<()> {
    let Some(path) = cache_path() else {
        anyhow::bail!("Could not determine cache path");
    };
    ensure_cache_dir(&path)?;
    let cache = plugin_cache(plugins);
    let content = serde_json::to_string(&cache)?;
    std::fs::write(&path, content)?;
    log::info!("Plugin cache written to {:?}", path);
    Ok(())
}

fn ensure_cache_dir(path: &std::path::Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)?;
    Ok(())
}

fn plugin_cache(plugins: &[PluginMetadata]) -> PluginCache {
    PluginCache {
        format_version: CACHE_FORMAT_VERSION,
        timestamp: current_timestamp(),
        plugins: plugins.iter().cloned().map(CachedPlugin::from).collect(),
    }
}

pub(crate) fn update_cached_version(plugin_id: &str, version: &str) {
    let Some(mut cache) = read_cache() else {
        return;
    };
    let Some(plugin) = cache
        .plugins
        .iter_mut()
        .find(|plugin| plugin.id == plugin_id)
    else {
        return;
    };
    let Some(path) = cache_path() else {
        return;
    };

    plugin.version = version.to_string();
    let Ok(content) = serde_json::to_string(&cache) else {
        return;
    };
    let _ = std::fs::write(path, content);
    log::info!("Updated cache version for {}: {}", plugin_id, version);
}
