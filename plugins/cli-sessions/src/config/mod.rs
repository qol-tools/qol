use serde::{Deserialize, Serialize};

use crate::storage::paths::PLUGIN_ID;
use crate::ui::placement::Corner;

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();
pub(crate) type ConfigInspection = qol_config::PluginConfigInspection<CliSessionsConfig>;

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct CliSessionsConfig {
    pub corner: Option<String>,
    pub service_commands: Vec<String>,
}

impl CliSessionsConfig {
    pub fn corner(&self) -> Corner {
        Corner::parse(self.corner.as_deref().unwrap_or_default())
    }
}

pub fn load() -> CliSessionsConfig {
    qol_config::load_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

pub(crate) fn inspect() -> Result<ConfigInspection, qol_config::PluginConfigInspectionError> {
    qol_config::inspect_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

#[cfg(test)]
fn contract_defaults() -> CliSessionsConfig {
    qol_config::typed_defaults_from_contract(CONFIG_CONTRACT).expect("contract defaults must parse")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_defaults_match_runtime_type() {
        qol_config::validate_contract_defaults_match_type::<CliSessionsConfig>(CONFIG_CONTRACT)
            .unwrap();

        let defaults = contract_defaults();
        assert_eq!(defaults.corner.as_deref(), Some("top-right"));
    }
}
