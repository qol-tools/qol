use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

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
    read_plugin_configs_from_dirs(
        &crate::paths::profile_plugin_configs_dir()?,
        &crate::paths::plugins_dir()?,
    )
}

pub fn replace_plugin_configs(configs: &HashMap<String, Value>) -> Result<()> {
    ensure_profile_dirs()?;
    replace_plugin_configs_in_dir(&crate::paths::profile_plugin_configs_dir()?, configs)
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
    let cached_urls = cached_repo_urls();
    let lock = build_plugins_lock(plugins, &existing, &cached_urls);
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

fn build_plugins_lock<'a>(
    plugins: impl IntoIterator<Item = &'a crate::plugins::Plugin>,
    existing: &PluginsLock,
    cached_urls: &HashMap<String, String>,
) -> PluginsLock {
    let existing_urls = existing_repo_urls(existing);
    let mut next = plugins
        .into_iter()
        .filter(|plugin| plugin.source == crate::plugins::PluginSource::Installed)
        .map(|plugin| PluginLockEntry {
            id: plugin.id.to_string(),
            repo_url: resolve_repo_url(plugin.id.as_str(), &existing_urls, cached_urls),
            version: plugin.manifest.plugin.version.clone(),
            platforms: plugin.manifest.plugin.platforms.clone(),
        })
        .collect::<Vec<_>>();
    next.extend(
        existing_urls
            .keys()
            .filter_map(|plugin_id| preserved_unsupported_entry(plugin_id.as_str(), existing)),
    );
    next.sort_by(|left, right| left.id.cmp(&right.id));
    next.dedup_by(|left, right| left.id == right.id);

    PluginsLock {
        version: CURRENT_PROFILE_VERSION,
        plugins: next,
    }
}

fn existing_repo_urls(existing: &PluginsLock) -> HashMap<String, String> {
    existing
        .plugins
        .iter()
        .map(|entry| (entry.id.clone(), entry.repo_url.clone()))
        .collect()
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

fn read_plugin_configs_from_dirs(
    profile_configs_dir: &Path,
    plugins_dir: &Path,
) -> Result<HashMap<String, Value>> {
    let mut configs = read_profile_plugin_configs_from_dir(profile_configs_dir)?;
    if !configs.is_empty() {
        return Ok(configs);
    }
    for (plugin_id, config) in read_installed_plugin_configs_from_dir(plugins_dir)? {
        configs.insert(plugin_id, config);
    }
    Ok(configs)
}

fn replace_plugin_configs_in_dir(
    profile_configs_dir: &Path,
    configs: &HashMap<String, Value>,
) -> Result<()> {
    clear_plugin_configs_dir(profile_configs_dir)?;
    for (plugin_id, config) in configs {
        write_plugin_config_in_dir(profile_configs_dir, plugin_id, config)?;
    }
    Ok(())
}

fn clear_plugin_configs_dir(dir: &Path) -> Result<()> {
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

fn read_profile_plugin_configs_from_dir(dir: &Path) -> Result<HashMap<String, Value>> {
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

fn read_installed_plugin_configs_from_dir(plugins_dir: &Path) -> Result<Vec<(String, Value)>> {
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

fn write_plugin_config_in_dir(
    profile_configs_dir: &Path,
    plugin_id: &str,
    config: &Value,
) -> Result<()> {
    if !crate::paths::is_safe_path_component(plugin_id) {
        anyhow::bail!("invalid plugin id");
    }
    crate::file_io::write_pretty_json(
        &profile_configs_dir.join(format!("{}.json", plugin_id)),
        config,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::manifest::{Capabilities, MenuConfig, PluginInfo};
    use crate::plugins::{Plugin, PluginId, PluginManifest, PluginSource};
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

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

    #[test]
    fn build_plugins_lock_preserves_unsupported_entries_and_resolves_repo_sources() {
        let existing = PluginsLock {
            version: CURRENT_PROFILE_VERSION,
            plugins: vec![
                PluginLockEntry {
                    id: "plugin-existing".to_string(),
                    repo_url: "https://example.com/existing.git".to_string(),
                    version: "0.1.0".to_string(),
                    platforms: None,
                },
                PluginLockEntry {
                    id: "plugin-unsupported".to_string(),
                    repo_url: "https://example.com/unsupported.git".to_string(),
                    version: "9.9.9".to_string(),
                    platforms: Some(vec![other_platform().to_string()]),
                },
                PluginLockEntry {
                    id: "plugin-missing".to_string(),
                    repo_url: "https://example.com/missing.git".to_string(),
                    version: "4.5.6".to_string(),
                    platforms: None,
                },
            ],
        };
        let cached_urls = HashMap::from([(
            "plugin-cached".to_string(),
            "https://example.com/cached.git".to_string(),
        )]);
        let plugins = [
            test_plugin("plugin-existing", "1.2.3", PluginSource::Installed, None),
            test_plugin("plugin-cached", "2.0.0", PluginSource::Installed, None),
            test_plugin("plugin-default", "3.0.0", PluginSource::Installed, None),
            test_plugin("plugin-dev", "8.8.8", PluginSource::DevLinked, None),
        ];

        let lock = build_plugins_lock(plugins.iter(), &existing, &cached_urls);
        let ids = lock
            .plugins
            .iter()
            .map(|plugin| plugin.id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            ids,
            vec![
                "plugin-cached",
                "plugin-default",
                "plugin-existing",
                "plugin-unsupported",
            ]
        );
        assert_eq!(
            repo_url_for(&lock, "plugin-existing"),
            "https://example.com/existing.git"
        );
        assert_eq!(
            repo_url_for(&lock, "plugin-cached"),
            "https://example.com/cached.git"
        );
        assert_eq!(
            repo_url_for(&lock, "plugin-default"),
            "https://github.com/qol-tools/plugin-default.git"
        );
        assert_eq!(version_for(&lock, "plugin-existing"), "1.2.3");
        assert_eq!(version_for(&lock, "plugin-unsupported"), "9.9.9");
        assert!(!lock
            .plugins
            .iter()
            .any(|plugin| plugin.id == "plugin-missing"));
        assert!(!lock.plugins.iter().any(|plugin| plugin.id == "plugin-dev"));
    }

    #[test]
    fn read_plugin_configs_from_dirs_cases() {
        struct Case {
            name: &'static str,
            profile_configs: Vec<(&'static str, Value)>,
            installed_configs: Vec<(&'static str, Value)>,
            expected: HashMap<String, Value>,
        }

        let cases = vec![
            Case {
                name: "profile wins over installed",
                profile_configs: vec![("plugin-test", json!({"source": "profile"}))],
                installed_configs: vec![
                    ("plugin-test", json!({"source": "installed"})),
                    ("plugin-extra", json!({"source": "installed"})),
                ],
                expected: HashMap::from([(
                    "plugin-test".to_string(),
                    json!({"source": "profile"}),
                )]),
            },
            Case {
                name: "installed configs are used when profile is empty",
                profile_configs: Vec::new(),
                installed_configs: vec![
                    ("plugin-a", json!({"enabled": true})),
                    ("plugin-b", json!({"count": 2})),
                ],
                expected: HashMap::from([
                    ("plugin-a".to_string(), json!({"enabled": true})),
                    ("plugin-b".to_string(), json!({"count": 2})),
                ]),
            },
        ];

        for case in cases {
            let tmp = TempDir::new().unwrap();
            let profile_configs_dir = tmp.path().join("profile");
            let plugins_dir = tmp.path().join("plugins");
            fs::create_dir_all(&profile_configs_dir).unwrap();

            for (plugin_id, config) in case.profile_configs {
                write_plugin_config_in_dir(&profile_configs_dir, plugin_id, &config).unwrap();
            }
            for (plugin_id, config) in case.installed_configs {
                write_installed_plugin_config(&plugins_dir, plugin_id, &config);
            }

            let configs =
                read_plugin_configs_from_dirs(&profile_configs_dir, &plugins_dir).unwrap();

            assert_eq!(configs, case.expected, "case: {}", case.name);
        }
    }

    #[test]
    fn read_profile_plugin_configs_from_dir_filters_invalid_entries() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path();
        write_plugin_config_in_dir(dir, "plugin-valid", &json!({"ok": true})).unwrap();
        fs::write(dir.join("plugin-ignored.txt"), "{}").unwrap();
        fs::write(dir.join("bad name.json"), "{}").unwrap();
        fs::write(dir.join("plugin-broken.json"), "{").unwrap();
        fs::create_dir_all(dir.join("plugin-dir.json")).unwrap();

        let configs = read_profile_plugin_configs_from_dir(dir).unwrap();

        assert_eq!(
            configs,
            HashMap::from([("plugin-valid".to_string(), json!({"ok": true}))])
        );
    }

    #[test]
    fn read_installed_plugin_configs_from_dir_filters_invalid_entries() {
        let tmp = TempDir::new().unwrap();
        let plugins_dir = tmp.path();
        write_installed_plugin_config(plugins_dir, "plugin-valid", &json!({"ok": true}));
        fs::create_dir_all(plugins_dir.join("plugin-no-config")).unwrap();
        fs::create_dir_all(plugins_dir.join("bad name")).unwrap();
        fs::write(
            crate::plugins::paths::config_path(&plugins_dir.join("bad name")),
            "{}",
        )
        .unwrap();
        fs::create_dir_all(plugins_dir.join("plugin-broken")).unwrap();
        fs::write(
            crate::plugins::paths::config_path(&plugins_dir.join("plugin-broken")),
            "{",
        )
        .unwrap();
        fs::write(plugins_dir.join("plain-file"), "x").unwrap();

        let configs = read_installed_plugin_configs_from_dir(plugins_dir)
            .unwrap()
            .into_iter()
            .collect::<HashMap<_, _>>();

        assert_eq!(
            configs,
            HashMap::from([("plugin-valid".to_string(), json!({"ok": true}))])
        );
    }

    #[test]
    fn replace_plugin_configs_removes_stale_profile_entries() {
        let tmp = TempDir::new().unwrap();
        let profile_configs_dir = tmp.path().join("profile");
        fs::create_dir_all(&profile_configs_dir).unwrap();
        write_plugin_config_in_dir(&profile_configs_dir, "plugin-old", &json!({"stale": true}))
            .unwrap();

        replace_plugin_configs_in_dir(
            &profile_configs_dir,
            &HashMap::from([("plugin-new".to_string(), json!({"fresh": true}))]),
        )
        .unwrap();

        assert!(!profile_configs_dir.join("plugin-old.json").exists());
        assert_eq!(
            crate::file_io::read_json::<Value>(&profile_configs_dir.join("plugin-new.json"))
                .unwrap(),
            json!({"fresh": true})
        );
    }

    #[test]
    fn replace_plugin_configs_rejects_invalid_plugin_ids() {
        let invalid_ids = ["../bad", "bad name", "", "."];

        for plugin_id in invalid_ids {
            let tmp = TempDir::new().unwrap();
            let profile_configs_dir = tmp.path().join("profile");
            fs::create_dir_all(&profile_configs_dir).unwrap();

            let result = replace_plugin_configs_in_dir(
                &profile_configs_dir,
                &HashMap::from([(plugin_id.to_string(), json!({"bad": true}))]),
            );

            assert!(result.is_err(), "plugin_id: {plugin_id}");
        }
    }

    fn test_plugin(
        id: &str,
        version: &str,
        source: PluginSource,
        platforms: Option<Vec<String>>,
    ) -> Plugin {
        Plugin::new_with_source(
            PluginId::new(id),
            PluginManifest {
                manifest_version: crate::plugins::manifest::CURRENT_MANIFEST_VERSION,
                plugin: PluginInfo {
                    name: id.to_string(),
                    description: String::new(),
                    version: version.to_string(),
                    author: None,
                    platforms,
                },
                menu: MenuConfig {
                    label: id.to_string(),
                    icon: None,
                    items: Vec::new(),
                },
                daemon: None,
                dependencies: None,
                runtime: None,
                capabilities: Capabilities::default(),
                build: Default::default(),
            },
            PathBuf::from(format!("/tmp/{id}")),
            source,
        )
    }

    fn repo_url_for<'a>(lock: &'a PluginsLock, plugin_id: &str) -> &'a str {
        lock.plugins
            .iter()
            .find(|plugin| plugin.id == plugin_id)
            .map(|plugin| plugin.repo_url.as_str())
            .unwrap()
    }

    fn version_for<'a>(lock: &'a PluginsLock, plugin_id: &str) -> &'a str {
        lock.plugins
            .iter()
            .find(|plugin| plugin.id == plugin_id)
            .map(|plugin| plugin.version.as_str())
            .unwrap()
    }

    fn write_installed_plugin_config(plugins_dir: &Path, plugin_id: &str, config: &Value) {
        let plugin_dir = plugins_dir.join(plugin_id);
        fs::create_dir_all(&plugin_dir).unwrap();
        crate::file_io::write_pretty_json(&crate::plugins::paths::config_path(&plugin_dir), config)
            .unwrap();
    }

    fn other_platform() -> &'static str {
        ["linux", "macos", "windows"]
            .into_iter()
            .find(|platform| *platform != std::env::consts::OS)
            .unwrap()
    }
}
