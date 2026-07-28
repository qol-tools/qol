use super::super::super::source::PluginSource;
use anyhow::Result;
use serde::{Deserialize, Serialize};

pub(in crate::features::plugin_store::github) fn build_plugin_metadata(
    plugin_dir: &str,
    source: &PluginSource,
    manifest: crate::plugins::PluginManifest,
    version: String,
) -> Result<PluginMetadata> {
    let plugin_id = manifest.plugin.require_declared_id()?.as_str().to_string();
    Ok(PluginMetadata {
        id: plugin_id,
        name: manifest.plugin.name,
        description: manifest.plugin.description,
        version,
        repo_url: source.plugin_subdir_html_url(plugin_dir),
        platforms: manifest.plugin.platforms,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PluginMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub repo_url: String,
    pub platforms: Option<Vec<String>>,
}

impl PluginMetadata {
    pub(crate) fn supports_current_platform(&self) -> bool {
        crate::plugins::manifest::supports_current_platform(&self.platforms)
    }

    #[cfg(test)]
    pub(crate) fn test_fixture(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        version: impl Into<String>,
        repo_url: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            version: version.into(),
            repo_url: repo_url.into(),
            platforms: None,
        }
    }
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

impl CachedPlugin {
    #[cfg(test)]
    pub(crate) fn test_fixture(
        id: impl Into<String>,
        name: impl Into<String>,
        description: impl Into<String>,
        version: impl Into<String>,
        repo_url: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            description: description.into(),
            version: version.into(),
            repo_url: repo_url.into(),
            platforms: None,
        }
    }
}
