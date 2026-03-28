use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub const CURRENT_PROFILE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileManifest {
    #[serde(default = "default_profile_version")]
    pub version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginLockEntry {
    pub id: String,
    pub repo_url: String,
    #[serde(default)]
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platforms: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginsLock {
    #[serde(default = "default_profile_version")]
    pub version: u32,
    #[serde(default)]
    pub plugins: Vec<PluginLockEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileExportBundle {
    #[serde(default = "default_profile_version")]
    pub version: u32,
    pub exported_at: String,
    pub hotkeys: Value,
    pub shortcuts: Value,
    pub task_runner: Value,
    #[serde(default)]
    pub plugin_configs: HashMap<String, Value>,
    #[serde(default)]
    pub plugins: Vec<PluginLockEntry>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProfileImportBundle {
    #[serde(default)]
    pub hotkeys: Option<Value>,
    #[serde(default)]
    pub shortcuts: Option<Value>,
    #[serde(default)]
    pub task_runner: Option<Value>,
    #[serde(default)]
    pub plugin_configs: Option<HashMap<String, Value>>,
    #[serde(default)]
    pub plugins: Vec<PluginLockEntry>,
    #[serde(default)]
    pub installed_plugins: Vec<String>,
}

fn default_profile_version() -> u32 {
    CURRENT_PROFILE_VERSION
}

pub fn ensure_profile_dirs() -> Result<()> {
    for path in [
        crate::paths::profile_dir()?,
        crate::paths::profile_core_dir()?,
        crate::paths::profile_plugin_configs_dir()?,
    ] {
        std::fs::create_dir_all(path)?;
    }
    save_manifest(&ProfileManifest {
        version: CURRENT_PROFILE_VERSION,
    })
}

pub fn load_manifest() -> Result<ProfileManifest> {
    let path = crate::paths::profile_manifest_path()?;
    if !path.exists() {
        return Ok(ProfileManifest {
            version: CURRENT_PROFILE_VERSION,
        });
    }
    crate::file_io::read_json(&path)
}

pub fn save_manifest(manifest: &ProfileManifest) -> Result<()> {
    crate::file_io::write_pretty_json(&crate::paths::profile_manifest_path()?, manifest)
}

pub fn load_plugins_lock() -> Result<PluginsLock> {
    let path = crate::paths::profile_plugins_lock_path()?;
    if !path.exists() {
        return Ok(PluginsLock {
            version: CURRENT_PROFILE_VERSION,
            plugins: Vec::new(),
        });
    }
    crate::file_io::read_json(&path)
}

pub fn save_plugins_lock(lock: &PluginsLock) -> Result<()> {
    ensure_profile_dirs()?;
    crate::file_io::write_pretty_json(&crate::paths::profile_plugins_lock_path()?, lock)
}

pub fn load_plugin_config(plugin_id: &str) -> Result<Option<Value>> {
    let path = crate::paths::profile_plugin_config_path(plugin_id)?;
    if !path.exists() {
        return Ok(None);
    }
    crate::file_io::read_json(&path).map(Some)
}

pub fn save_plugin_config(plugin_id: &str, config: &Value) -> Result<()> {
    ensure_profile_dirs()?;
    crate::file_io::write_pretty_json(
        &crate::paths::profile_plugin_config_path(plugin_id)?,
        config,
    )
}

pub fn read_plugin_configs() -> Result<HashMap<String, Value>> {
    ensure_profile_dirs()?;
    let mut configs = read_profile_plugin_configs()?;
    if !configs.is_empty() {
        return Ok(configs);
    }
    for (plugin_id, config) in read_installed_plugin_configs()? {
        configs.insert(plugin_id, config);
    }
    Ok(configs)
}

pub fn replace_plugin_configs(configs: &HashMap<String, Value>) -> Result<()> {
    ensure_profile_dirs()?;
    clear_plugin_configs_dir()?;
    for (plugin_id, config) in configs {
        save_plugin_config(plugin_id, config)?;
    }
    Ok(())
}

pub fn import_plugins(bundle: &ProfileImportBundle) -> Vec<PluginLockEntry> {
    if !bundle.plugins.is_empty() {
        return bundle.plugins.clone();
    }

    bundle
        .installed_plugins
        .iter()
        .map(|plugin_id| PluginLockEntry {
            id: plugin_id.clone(),
            repo_url: default_repo_url(plugin_id),
            version: String::new(),
            platforms: None,
        })
        .collect()
}

pub fn sync_plugins_lock_from_plugins<'a>(
    plugins: impl IntoIterator<Item = &'a crate::plugins::Plugin>,
) -> Result<PluginsLock> {
    ensure_profile_dirs()?;
    let existing = load_plugins_lock().unwrap_or(PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: Vec::new(),
    });
    let existing_urls = existing
        .plugins
        .iter()
        .map(|entry| (entry.id.clone(), entry.repo_url.clone()))
        .collect::<HashMap<_, _>>();
    let cached_urls = cached_repo_urls();

    let mut next = plugins
        .into_iter()
        .filter(|plugin| plugin.source == crate::plugins::PluginSource::Installed)
        .map(|plugin| PluginLockEntry {
            id: plugin.id.to_string(),
            repo_url: resolve_repo_url(plugin.id.as_str(), &existing_urls, &cached_urls),
            version: plugin.manifest.plugin.version.clone(),
            platforms: plugin.manifest.plugin.platforms.clone(),
        })
        .collect::<Vec<_>>();
    next.extend(
        existing_urls
            .keys()
            .filter_map(|plugin_id| preserved_unsupported_entry(plugin_id.as_str(), &existing)),
    );
    next.sort_by(|left, right| left.id.cmp(&right.id));
    next.dedup_by(|left, right| left.id == right.id);

    let lock = PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: next,
    };
    save_plugins_lock(&lock)?;
    Ok(lock)
}

fn cached_repo_urls() -> HashMap<String, String> {
    let Some(cache) = crate::features::plugin_store::github::read_cache() else {
        return HashMap::new();
    };
    cache
        .plugins
        .into_iter()
        .map(|plugin| (plugin.id, plugin.repo_url))
        .collect()
}

fn resolve_repo_url(
    plugin_id: &str,
    existing_urls: &HashMap<String, String>,
    cached_urls: &HashMap<String, String>,
) -> String {
    if let Some(repo_url) = existing_urls.get(plugin_id) {
        return repo_url.clone();
    }
    if let Some(repo_url) = cached_urls.get(plugin_id) {
        return repo_url.clone();
    }
    default_repo_url(plugin_id)
}

fn default_repo_url(plugin_id: &str) -> String {
    format!("https://github.com/qol-tools/{}.git", plugin_id)
}

fn preserved_unsupported_entry(plugin_id: &str, existing: &PluginsLock) -> Option<PluginLockEntry> {
    let entry = existing
        .plugins
        .iter()
        .find(|entry| entry.id == plugin_id)?;
    if crate::plugins::manifest::supports_current_platform(&entry.platforms) {
        return None;
    }
    Some(entry.clone())
}

fn clear_plugin_configs_dir() -> Result<()> {
    let dir = crate::paths::profile_plugin_configs_dir()?;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !crate::paths::is_safe_path_component(stem) {
            continue;
        }
        std::fs::remove_file(path)?;
    }
    Ok(())
}

fn read_profile_plugin_configs() -> Result<HashMap<String, Value>> {
    let dir = crate::paths::profile_plugin_configs_dir()?;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(HashMap::new());
    };

    let mut configs = HashMap::new();
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !crate::paths::is_safe_path_component(stem) {
            continue;
        }
        let Ok(config) = crate::file_io::read_json::<Value>(&path) else {
            continue;
        };
        configs.insert(stem.to_string(), config);
    }
    Ok(configs)
}

fn read_installed_plugin_configs() -> Result<Vec<(String, Value)>> {
    let plugins_dir = crate::paths::plugins_dir()?;
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return Ok(Vec::new());
    };
    let mut configs = Vec::new();
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let file_name = entry.file_name();
        let Some(plugin_id) = file_name.to_str().map(String::from) else {
            continue;
        };
        if !crate::paths::is_safe_path_component(&plugin_id) {
            continue;
        }
        let config_path = crate::plugins::paths::config_path(&path);
        if !config_path.exists() {
            continue;
        }
        let Ok(config) = crate::file_io::read_json::<Value>(&config_path) else {
            continue;
        };
        configs.push((plugin_id, config));
    }
    Ok(configs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_plugins_prefers_explicit_lock_entries() {
        let bundle = ProfileImportBundle {
            plugins: vec![PluginLockEntry {
                id: "plugin-test".to_string(),
                repo_url: "https://github.com/qol-tools/plugin-test".to_string(),
                version: "1.2.3".to_string(),
                platforms: Some(vec!["linux".to_string()]),
            }],
            installed_plugins: vec!["plugin-ignored".to_string()],
            plugin_configs: Some(HashMap::new()),
            ..ProfileImportBundle::default()
        };

        let plugins = import_plugins(&bundle);

        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "plugin-test");
        assert_eq!(plugins[0].version, "1.2.3");
    }

    #[test]
    fn import_plugins_falls_back_to_legacy_installed_plugins() {
        let bundle = ProfileImportBundle {
            installed_plugins: vec!["plugin-test".to_string()],
            ..ProfileImportBundle::default()
        };

        let plugins = import_plugins(&bundle);

        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].id, "plugin-test");
        assert_eq!(
            plugins[0].repo_url,
            "https://github.com/qol-tools/plugin-test.git"
        );
        assert!(plugins[0].version.is_empty());
    }
}
