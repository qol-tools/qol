use super::PluginConfigs;
use anyhow::Result;
use std::path::Path;

pub(super) fn load_configs(config_path: &Path) -> Result<PluginConfigs> {
    if !config_path.exists() {
        return Ok(PluginConfigs::default());
    }

    let content = std::fs::read_to_string(config_path)?;
    let configs = serde_json::from_str(&content)?;
    Ok(configs)
}

pub(super) fn save_configs(config_path: &Path, configs: &PluginConfigs) -> Result<()> {
    ensure_parent_dir(config_path)?;
    let content = serde_json::to_string_pretty(configs)?;
    std::fs::write(config_path, content)?;
    Ok(())
}

pub(super) fn load_plugin_config(plugin_path: &Path) -> Result<serde_json::Value> {
    let content = std::fs::read_to_string(plugin_path)?;
    let config = serde_json::from_str(&content)?;
    Ok(config)
}

pub(super) fn write_plugin_config(plugin_path: &Path, config: &serde_json::Value) -> Result<()> {
    ensure_parent_dir(plugin_path)?;
    let content = serde_json::to_string_pretty(config)?;
    std::fs::write(plugin_path, content)?;
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)?;
    Ok(())
}
