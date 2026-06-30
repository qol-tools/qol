use anyhow::{Error, Result};

use super::model::PluginConfig;

pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();

pub fn load() -> Result<PluginConfig> {
    let overrides = qol_runtime::plugin_config::load_json()
        .filter(|value| !value.is_null())
        .unwrap_or_else(|| serde_json::json!({}));
    let config: PluginConfig =
        qol_config::deserialize_with_contract_defaults(CONFIG_CONTRACT, overrides)
            .map_err(format_config_errors)?;
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

fn format_config_errors(errors: Vec<qol_config::validation::ValidationError>) -> Error {
    Error::msg(
        errors
            .into_iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; "),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_defaults_match_runtime_type() {
        qol_config::validate_contract_defaults_match_type::<PluginConfig>(CONFIG_CONTRACT).unwrap();

        let defaults: PluginConfig =
            qol_config::typed_defaults_from_contract(CONFIG_CONTRACT).unwrap();
        assert_eq!(defaults.live_color_hex, "#FFFFFF");
        assert_eq!(defaults.live_brightness, 100);
    }
}
