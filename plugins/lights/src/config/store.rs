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

pub fn inspect() -> Result<qol_config::PluginConfigInspection<PluginConfig>> {
    let inspection =
        qol_config::inspect_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)?;
    super::validation::validate(&inspection.config).map_err(Error::msg)?;
    Ok(inspection)
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
        assert_eq!(defaults.live_mirek, 300);
        assert_eq!(defaults.main_target_id, "");
        assert_eq!(defaults.backend.serial_port, "auto");
        assert_eq!(defaults.backend.channel, 11);
        assert_eq!(defaults.presets.preset_8.name, "Preset 8");
        assert_eq!(defaults.presets.preset_8.color_hex, "#FFFFFF");
        assert!(!defaults.presets.preset_1.enabled);
    }

    #[test]
    fn contract_declares_every_preset_slot_field() {
        let spec = qol_config::contract::parse_spec_str(CONFIG_CONTRACT).unwrap();
        for slot in 1..=8 {
            for key in [
                "enabled",
                "name",
                "power_on",
                "brightness",
                "color_hex",
                "mirek",
            ] {
                let id = format!("preset_{slot}_{key}");
                let field = spec.field(&id).unwrap_or_else(|| panic!("missing {id}"));
                assert_eq!(
                    field.config_key.as_deref(),
                    Some(format!("presets.preset_{slot}.{key}").as_str())
                );
            }
        }
    }
}
