use super::HotkeyConfig;
use anyhow::Result;
use std::path::Path;

pub(super) fn load_config(config_path: &Path) -> Result<HotkeyConfig> {
    if !config_path.exists() {
        return Ok(HotkeyConfig::default());
    }

    let content = std::fs::read_to_string(config_path)?;
    let config = serde_json::from_str(&content)?;
    Ok(config)
}

pub(super) fn save_config(config_path: &Path, config: &HotkeyConfig) -> Result<()> {
    ensure_parent_dir(config_path)?;
    let content = serde_json::to_string_pretty(config)?;
    std::fs::write(config_path, content)?;
    Ok(())
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)?;
    Ok(())
}
