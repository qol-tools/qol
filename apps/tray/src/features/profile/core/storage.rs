use super::{
    PluginLockEntry, PluginsLock, ProfileImportBundle, ProfileManifest, CURRENT_PROFILE_VERSION,
};
use crate::plugins::PluginUid;
use anyhow::{bail, Result};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

pub fn ensure_profile_dirs() -> Result<()> {
    let name = crate::paths::active_profile_name();
    super::super::registry::ensure_profile_dirs_for(&name)?;
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
    update_plugins_lock(|_| Ok(lock.clone())).map(|_| ())
}

pub(crate) fn update_plugins_lock<F>(build: F) -> Result<(PluginsLock, u64)>
where
    F: FnOnce(&PluginsLock) -> Result<PluginsLock>,
{
    let _mutation = crate::plugins::config::begin_runtime_config_mutation_for_active_profile()?;
    let profile_guard = crate::plugins::config::profile_config_write_guard_unmarked();
    ensure_profile_dirs()?;
    let (previous, fallback) = match load_plugins_lock() {
        Ok(previous) => (previous, false),
        Err(_) => (
            PluginsLock {
                version: CURRENT_PROFILE_VERSION,
                plugins: Vec::new(),
            },
            true,
        ),
    };
    let lock = build(&previous)?;
    let scope = if fallback {
        crate::plugins::config::ProfileConfigInvalidation::All
    } else {
        crate::plugins::config::ProfileConfigInvalidation::Plugins(changed_plugin_ids(
            &previous, &lock,
        ))
    };
    let generation = profile_guard.mark_changed(scope);
    crate::file_io::write_pretty_json(&crate::paths::profile_plugins_lock_path()?, &lock)?;
    Ok((lock, generation))
}

pub(super) fn changed_plugin_ids(previous: &PluginsLock, next: &PluginsLock) -> Vec<String> {
    let previous = previous
        .plugins
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect::<HashMap<_, _>>();
    let next = next
        .plugins
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect::<HashMap<_, _>>();
    previous
        .keys()
        .chain(next.keys())
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter(|plugin_id| previous.get(plugin_id) != next.get(plugin_id))
        .map(str::to_string)
        .collect()
}

pub(super) struct PluginUidIndex {
    id_to_uid: HashMap<String, PluginUid>,
    uid_to_id: HashMap<String, String>,
    known_uids: HashSet<String>,
}

impl PluginUidIndex {
    pub(super) fn from_plugins(
        plugins_dir: &Path,
        plugins: &[PluginLockEntry],
        stored: &PluginsLock,
    ) -> Self {
        let bundle_ids = plugins
            .iter()
            .map(|plugin| plugin.id.as_str())
            .collect::<HashSet<_>>();
        let mut id_to_uid = HashMap::new();
        let mut uid_to_id = HashMap::new();
        let mut known_uids = HashSet::new();
        for entry in plugins.iter().chain(
            stored
                .plugins
                .iter()
                .filter(|entry| !bundle_ids.contains(entry.id.as_str())),
        ) {
            if !crate::paths::is_safe_path_component(&entry.id)
                || !crate::paths::is_safe_path_component(entry.uid.as_str())
            {
                continue;
            }
            let uid = resolved_entry_uid(plugins_dir, entry);
            if !crate::paths::is_safe_path_component(uid.as_str()) {
                continue;
            }
            if id_to_uid.contains_key(&entry.id) || known_uids.contains(uid.as_str()) {
                continue;
            }
            id_to_uid.insert(entry.id.clone(), uid.clone());
            uid_to_id.insert(uid.as_str().to_string(), entry.id.clone());
            known_uids.insert(uid.as_str().to_string());
        }
        Self {
            id_to_uid,
            uid_to_id,
            known_uids,
        }
    }

    pub(super) fn resolve_stem(&self, stem: &str) -> String {
        if self.known_uids.contains(stem) {
            return stem.to_string();
        }
        match self.id_to_uid.get(stem) {
            Some(uid) => uid.as_str().to_string(),
            None => stem.to_string(),
        }
    }

    pub(super) fn resolve_plugin_id<'a>(&'a self, key: &'a str) -> &'a str {
        self.uid_to_id.get(key).map(String::as_str).unwrap_or(key)
    }

    pub(super) fn uid_for_id<'a>(&'a self, plugin_id: &'a str) -> &'a str {
        self.id_to_uid
            .get(plugin_id)
            .map(PluginUid::as_str)
            .unwrap_or(plugin_id)
    }
}

pub(super) fn resolved_entry_uid(plugins_dir: &Path, entry: &PluginLockEntry) -> PluginUid {
    if entry.uid.as_str() != entry.id.as_str() {
        return entry.uid.clone();
    }
    crate::plugins::PluginManifest::read_from_dir(plugins_dir.join(&entry.id))
        .ok()
        .and_then(|manifest| manifest.plugin.uid)
        .unwrap_or_else(|| entry.uid.clone())
}

pub(super) fn canonicalize_plugin_configs(
    configs: &HashMap<String, Value>,
    uid_index: &PluginUidIndex,
) -> HashMap<String, Value> {
    let mut canonical = HashMap::new();
    let mut aliases = Vec::new();
    let mut keys = configs.keys().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        let resolved = uid_index.resolve_stem(key);
        if resolved == key.as_str() {
            canonical.insert(resolved, configs[key].clone());
        } else {
            aliases.push((resolved, configs[key].clone()));
        }
    }
    for (key, config) in aliases {
        canonical.entry(key).or_insert(config);
    }
    canonical
}

pub fn read_plugin_configs(plugins: &[PluginLockEntry]) -> Result<HashMap<String, Value>> {
    ensure_profile_dirs()?;
    let installed_configs_dir = crate::paths::plugins_dir()?;
    let stored = load_plugins_lock().unwrap_or_else(|_| PluginsLock::empty());
    let uid_index = PluginUidIndex::from_plugins(&installed_configs_dir, plugins, &stored);
    read_plugin_configs_from_dirs(
        &crate::paths::profile_plugin_configs_dir()?,
        &installed_configs_dir,
        &uid_index,
    )
}

pub fn replace_plugin_configs(configs: &HashMap<String, Value>) -> Result<()> {
    let _profile_guard = crate::plugins::config::profile_config_write_guard();
    let _mutation = crate::plugins::config::begin_runtime_config_mutation_for_active_profile()?;
    ensure_profile_dirs()?;
    replace_plugin_configs_in_dir(&crate::paths::profile_plugin_configs_dir()?, configs)
}

pub fn read_hotkeys_list() -> Vec<Value> {
    read_wrapped_json_array(crate::paths::hotkeys_path(), "hotkeys")
}

pub fn read_shortcuts_list() -> Vec<Value> {
    read_wrapped_json_array(crate::paths::shortcuts_path(), "shortcuts")
}

pub fn read_task_runner_value() -> Value {
    read_json_file_or_default(crate::paths::task_runner_config_path())
}

pub(super) fn write_core_settings(bundle: &ProfileImportBundle) -> Result<()> {
    if let Some(hotkeys) = &bundle.hotkeys {
        write_json_config(
            crate::paths::hotkeys_path()?,
            &serde_json::json!({ "hotkeys": hotkeys }),
        )?;
    }
    if let Some(shortcuts) = &bundle.shortcuts {
        write_json_config(
            crate::paths::shortcuts_path()?,
            &serde_json::json!({ "shortcuts": shortcuts }),
        )?;
    }
    if let Some(task_runner) = &bundle.task_runner {
        write_json_config(crate::paths::task_runner_config_path()?, task_runner)?;
    }
    Ok(())
}

fn read_wrapped_json_array(path: Result<PathBuf>, field_name: &str) -> Vec<Value> {
    read_wrapped_json_array_value(read_json_file_or_default(path), field_name)
}

fn read_wrapped_json_array_value(value: Value, field_name: &str) -> Vec<Value> {
    if value.is_null() {
        return Vec::new();
    }
    if let Value::Array(items) = value {
        return items;
    }
    if let Value::Object(mut object) = value {
        let Some(items) = object.remove(field_name) else {
            return Vec::new();
        };
        if let Value::Array(items) = items {
            return items;
        }
    }
    Vec::new()
}

fn read_json_file_or_default(path: Result<PathBuf>) -> Value {
    let Ok(path) = path else {
        return Value::Null;
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(_) => return Value::Null,
    };
    serde_json::from_str(&content).unwrap_or(Value::Null)
}

fn write_json_config(path: PathBuf, value: &Value) -> Result<()> {
    let content = serde_json::to_vec_pretty(value)?;
    crate::file_io::ensure_parent_dir(&path)?;
    crate::file_io::atomic_write(&path, &content)
}

pub(super) fn read_plugin_configs_from_dirs(
    profile_configs_dir: &Path,
    plugins_dir: &Path,
    uid_index: &PluginUidIndex,
) -> Result<HashMap<String, Value>> {
    let mut configs = HashMap::new();
    let mut id_resolved = Vec::new();
    for (stem, config) in read_profile_plugin_configs_from_dir(profile_configs_dir)? {
        let key = uid_index.resolve_stem(&stem);
        if key == stem {
            configs.insert(key, config);
        } else {
            id_resolved.push((key, config));
        }
    }
    for (key, config) in id_resolved {
        configs.entry(key).or_insert(config);
    }
    for (plugin_id, config) in read_installed_plugin_configs_from_dir(plugins_dir)? {
        let key = uid_index.uid_for_id(&plugin_id).to_string();
        configs.entry(key).or_insert(config);
    }
    Ok(configs)
}

pub(super) fn validate_plugin_config_keys(configs: &HashMap<String, Value>) -> Result<()> {
    for key in configs.keys() {
        if !crate::paths::is_safe_path_component(key) {
            bail!("invalid plugin config key: {}", key);
        }
    }
    Ok(())
}

pub(super) fn replace_plugin_configs_in_dir(
    profile_configs_dir: &Path,
    configs: &HashMap<String, Value>,
) -> Result<()> {
    validate_plugin_config_keys(configs)?;
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

pub(super) fn read_profile_plugin_configs_from_dir(dir: &Path) -> Result<HashMap<String, Value>> {
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

pub(super) fn read_installed_plugin_configs_from_dir(
    plugins_dir: &Path,
) -> Result<Vec<(String, Value)>> {
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return Ok(Vec::new());
    };
    let mut configs = Vec::new();
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        let file_name = entry.file_name();
        let Some(plugin_id) = file_name.to_str().map(String::from) else {
            trace_installed_plugin_config_entry("", &path, "skip_non_utf8");
            continue;
        };
        if !crate::paths::is_safe_path_component(&plugin_id) {
            trace_installed_plugin_config_entry(&plugin_id, &path, "skip_invalid_id");
            continue;
        }
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            trace_installed_plugin_config_entry(&plugin_id, &path, "skip_missing");
            continue;
        };
        if metadata.file_type().is_symlink() {
            trace_installed_plugin_config_entry(&plugin_id, &path, "skip_symlink");
            continue;
        }
        if !metadata.is_dir() {
            trace_installed_plugin_config_entry(&plugin_id, &path, "skip_not_dir");
            continue;
        }
        let config_path = crate::plugins::paths::config_path(&path);
        if !config_path.exists() {
            trace_installed_plugin_config_entry(&plugin_id, &path, "skip_no_config");
            continue;
        }
        let Ok(config) = crate::file_io::read_json::<Value>(&config_path) else {
            trace_installed_plugin_config_entry(&plugin_id, &path, "skip_invalid_json");
            continue;
        };
        trace_installed_plugin_config_entry(&plugin_id, &path, "include");
        configs.push((plugin_id, config));
    }
    Ok(configs)
}

fn trace_installed_plugin_config_entry(plugin_id: &str, path: &Path, outcome: &str) {
    #[cfg(debug_assertions)]
    {
        qol_runtime::probe!(
            "PROFILE_CONFIG_SOURCE",
            "plugin={:?} entry_kind={} outcome={outcome}",
            plugin_id,
            trace_plugin_entry_kind(path)
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = (plugin_id, path, outcome);
}

#[cfg(debug_assertions)]
fn trace_plugin_entry_kind(path: &Path) -> &'static str {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return "missing";
    };
    if metadata.file_type().is_symlink() {
        return "symlink";
    }
    if metadata.is_dir() {
        return "dir";
    }
    if metadata.is_file() {
        return "file";
    }
    "other"
}

pub(super) fn write_plugin_config_in_dir(
    profile_configs_dir: &Path,
    plugin_id: &str,
    config: &Value,
) -> Result<()> {
    if !crate::paths::is_safe_path_component(plugin_id) {
        bail!("invalid plugin id");
    }
    crate::file_io::write_pretty_json(
        &profile_configs_dir.join(format!("{}.json", plugin_id)),
        config,
    )
}
