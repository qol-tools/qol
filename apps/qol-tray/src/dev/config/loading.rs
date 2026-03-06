use anyhow::Result;

use super::DevConfig;

pub fn load() -> Result<DevConfig> {
    let path = crate::paths::dev_config_path()?;
    if !path.exists() {
        return Ok(DevConfig::default());
    }

    let content = std::fs::read_to_string(&path)?;
    let config: DevConfig = serde_json::from_str(&content)?;
    Ok(config)
}
