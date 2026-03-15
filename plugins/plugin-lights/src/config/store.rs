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
    let content =
        std::fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let config: PluginConfig = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    super::validation::validate(&config).map_err(Error::msg)?;
    Ok(config)
}

pub fn save(config: &PluginConfig) -> Result<()> {
    super::validation::validate(config).map_err(Error::msg)?;
    let path = config_path()?;
    let json = serde_json::to_string_pretty(config)
        .with_context(|| format!("failed to serialize {}", path.display()))?;
    std::fs::write(&path, format!("{}\n", json))
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn config_path() -> Result<PathBuf> {
    std::env::current_dir()
        .context("failed to resolve current directory")
        .map(|path| path.join("config.json"))
}
