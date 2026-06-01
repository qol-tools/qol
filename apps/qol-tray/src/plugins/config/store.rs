use crate::features::profile::scope_store::PluginConfigSlicePaths;
use crate::file_io;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

use super::scope::ConfigSlices;

pub(super) fn load_configs(configs_dir: &Path) -> Result<HashMap<String, serde_json::Value>> {
    std::fs::create_dir_all(configs_dir)?;
    let mut configs = HashMap::new();
    let Ok(entries) = std::fs::read_dir(configs_dir) else {
        return Ok(configs);
    };

    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Some(plugin_id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !crate::paths::is_safe_path_component(plugin_id) {
            continue;
        }
        configs.insert(plugin_id.to_string(), file_io::read_json(&path)?);
    }

    Ok(configs)
}

pub(super) fn save_configs(
    configs_dir: &Path,
    configs: &HashMap<String, serde_json::Value>,
) -> Result<()> {
    std::fs::create_dir_all(configs_dir)?;
    clear_configs_dir(configs_dir)?;
    for (plugin_id, config) in configs {
        write_profile_plugin_config(configs_dir, plugin_id, config)?;
    }
    Ok(())
}

pub(super) fn load_plugin_config(plugin_path: &Path) -> Result<serde_json::Value> {
    file_io::read_json(plugin_path)
}

pub(super) fn write_plugin_config(plugin_path: &Path, config: &serde_json::Value) -> Result<()> {
    file_io::write_pretty_json(plugin_path, config)
}

pub(super) fn write_profile_plugin_config(
    configs_dir: &Path,
    plugin_id: &str,
    config: &serde_json::Value,
) -> Result<()> {
    let path = profile_plugin_config_path(configs_dir, plugin_id)?;
    file_io::write_pretty_json(&path, config)
}

fn profile_plugin_config_path(configs_dir: &Path, plugin_id: &str) -> Result<std::path::PathBuf> {
    if !crate::paths::is_safe_path_component(plugin_id) {
        anyhow::bail!("Invalid plugin ID: {}", plugin_id);
    }
    Ok(configs_dir.join(format!("{}.json", plugin_id)))
}

pub(super) fn read_scoped_slices(paths: &PluginConfigSlicePaths) -> Result<ConfigSlices> {
    Ok(ConfigSlices {
        core: read_optional_object(&paths.core)?,
        os: read_optional_object(&paths.os)?,
        device: read_optional_object(&paths.device)?,
    })
}

pub(super) fn write_scoped_slices(
    paths: &PluginConfigSlicePaths,
    slices: &ConfigSlices,
) -> Result<()> {
    write_slice_at(&paths.core, &slices.core)?;
    write_slice_at(&paths.os, &slices.os)?;
    write_slice_at(&paths.device, &slices.device)?;
    Ok(())
}

fn read_optional_object(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    file_io::read_json(path)
}

fn write_slice_at(path: &Path, value: &Value) -> Result<()> {
    let is_empty_object = value.as_object().is_some_and(|m| m.is_empty());
    if is_empty_object {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    file_io::write_pretty_json(path, value)
}

fn clear_configs_dir(configs_dir: &Path) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(configs_dir) else {
        return Ok(());
    };
    for entry in entries.filter_map(|entry| entry.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(plugin_id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !crate::paths::is_safe_path_component(plugin_id) {
            continue;
        }
        std::fs::remove_file(path)?;
    }
    Ok(())
}
