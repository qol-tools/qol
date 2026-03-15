use anyhow::{Context, Error, Result};
use std::path::PathBuf;

use super::model::PluginConfig;

pub fn load() -> Result<PluginConfig> {
    let path = best_config_path();
    if !path.is_file() {
        let config = PluginConfig::default();
        save(&config)?;
        return Ok(config);
    }
    let content =
        std::fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let config: PluginConfig = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    super::validation::validate(&config).map_err(Error::msg)?;
    Ok(config)
}

fn best_config_path() -> PathBuf {
    if let Some(mirror) = mirror_config_path() {
        if mirror.is_file() {
            return mirror;
        }
    }
    config_path().unwrap_or_else(|_| PathBuf::from("config.json"))
}

pub fn save(config: &PluginConfig) -> Result<()> {
    super::validation::validate(config).map_err(Error::msg)?;
    let json = serde_json::to_string_pretty(config)
        .context("failed to serialize config")?;
    let content = format!("{}\n", json);

    let path = config_path()?;
    std::fs::write(&path, &content)
        .with_context(|| format!("failed to write {}", path.display()))?;

    if let Some(mirror) = mirror_config_path() {
        if mirror != path {
            let _ = std::fs::create_dir_all(mirror.parent().unwrap());
            let _ = std::fs::write(&mirror, &content);
        }
    }

    Ok(())
}

fn mirror_config_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".config/qol-tray/plugins/plugin-lights/config.json"))
}

fn config_path() -> Result<PathBuf> {
    std::env::current_dir()
        .context("failed to resolve current directory")
        .map(|path| path.join("config.json"))
}
