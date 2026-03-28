use crate::file_io;
use anyhow::Result;
use std::collections::HashMap;
use std::path::Path;

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

pub(super) fn load_profile_plugin_config(
    configs_dir: &Path,
    plugin_id: &str,
) -> Result<Option<serde_json::Value>> {
    let path = profile_plugin_config_path(configs_dir, plugin_id)?;
    if !path.exists() {
        return Ok(None);
    }
    file_io::read_json(&path).map(Some)
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
