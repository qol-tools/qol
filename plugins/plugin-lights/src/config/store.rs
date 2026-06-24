use anyhow::{Error, Result};

use super::model::PluginConfig;

pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

pub fn load() -> Result<PluginConfig> {
    let config: PluginConfig = qol_runtime::plugin_config::load();
    super::validation::validate(&config).map_err(Error::msg)?;
    Ok(config)
}

pub fn save(config: &PluginConfig) -> Result<()> {
    super::validation::validate(config).map_err(Error::msg)?;
    if qol_runtime::plugin_config::save(config) {
        Ok(())
    } else {
        anyhow::bail!("{PLUGIN_ID}: failed to persist config over runtime socket")
    }
}
