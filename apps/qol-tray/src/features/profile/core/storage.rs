use super::{PluginsLock, ProfileImportBundle, ProfileManifest, CURRENT_PROFILE_VERSION};
use anyhow::{bail, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
) -> Result<HashMap<String, Value>> {
    let mut configs = read_installed_plugin_configs_from_dir(plugins_dir)?
        .into_iter()
        .collect::<HashMap<_, _>>();
    for (plugin_id, config) in read_profile_plugin_configs_from_dir(profile_configs_dir)? {
        configs.insert(plugin_id, config);
    }
    Ok(configs)
}

pub(super) fn replace_plugin_configs_in_dir(
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
