use anyhow::{Context, Error, Result};
use std::path::PathBuf;

use super::model::PluginConfig;

pub fn load() -> Result<PluginConfig> {
    let path = config_path()?;
    if !path.is_file() {
        let config = PluginConfig::default();
        save(&config)?;
        return Ok(config);
    }
    let content = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let config: PluginConfig = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    super::validation::validate(&config).map_err(Error::msg)?;
    Ok(config)
}

pub fn save(config: &PluginConfig) -> Result<()> {
    super::validation::validate(config).map_err(Error::msg)?;
    let json = serde_json::to_string_pretty(config).context("failed to serialize config")?;
    let content = format!("{}\n", json);

    let path = config_path()?;
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    std::fs::write(&path, &content)
        .with_context(|| format!("failed to write {}", path.display()))?;

    Ok(())
}

fn config_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME env var not set")?;
    Ok(PathBuf::from(home).join(".config/qol-tray/plugins/plugin-lights/config.json"))
}
