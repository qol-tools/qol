use serde::{Deserialize, Serialize};

use crate::storage::paths::PLUGIN_ID;
use crate::ui::placement::{parse_corner, Corner};

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();
pub(crate) type ConfigInspection = qol_config::PluginConfigInspection<CliSessionsConfig>;

pub fn contract() -> &'static str {
    CONFIG_CONTRACT
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(default)]
pub struct CliSessionsConfig {
    pub corner: Option<String>,
    pub service_commands: Vec<String>,
    pub switchboard_model: Option<String>,
    pub switchboard_effort: Option<String>,
    pub title_bar: bool,
}

impl CliSessionsConfig {
    pub fn corner(&self) -> Corner {
        parse_corner(self.corner.as_deref().unwrap_or_default())
    }

    pub fn switchboard_model(&self) -> &str {
        non_blank(self.switchboard_model.as_deref())
            .unwrap_or(qol_sessions_engine::switchboard::DEFAULT_MODEL)
    }

    pub fn switchboard_effort(&self) -> &str {
        non_blank(self.switchboard_effort.as_deref())
            .unwrap_or(qol_sessions_engine::switchboard::DEFAULT_EFFORT)
    }

    pub fn switchboard_profile(&self) -> qol_sessions_engine::switchboard::Profile {
        qol_sessions_engine::switchboard::Profile {
            model: self.switchboard_model().to_owned(),
            effort: self.switchboard_effort().to_owned(),
        }
    }
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
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
        assert!(
            defaults.service_commands.is_empty(),
            "service_commands defaults to an empty editable list"
        );
        assert_eq!(defaults.switchboard_model(), "claude-sonnet-5-5");
        assert_eq!(defaults.switchboard_effort(), "low");
    }

    #[test]
    fn blank_switchboard_fields_fall_back_to_the_profile_defaults() {
        let config = CliSessionsConfig {
            switchboard_model: Some("  ".to_owned()),
            switchboard_effort: Some(String::new()),
            ..CliSessionsConfig::default()
        };
        assert_eq!(config.switchboard_model(), "claude-sonnet-5-5");
        assert_eq!(config.switchboard_effort(), "low");

        let config = CliSessionsConfig {
            switchboard_model: Some("claude-opus-5-5".to_owned()),
            switchboard_effort: Some("medium".to_owned()),
            ..CliSessionsConfig::default()
        };
        assert_eq!(config.switchboard_model(), "claude-opus-5-5");
        assert_eq!(config.switchboard_effort(), "medium");
    }
}
