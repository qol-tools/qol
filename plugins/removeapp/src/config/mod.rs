use serde::{Deserialize, Serialize};

use crate::cli::PLUGIN_ID;

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct RemoveAppConfig {
    #[serde(default = "shows_title_bar")]
    pub title_bar: bool,
}

impl Default for RemoveAppConfig {
    fn default() -> Self {
        Self {
            title_bar: shows_title_bar(),
        }
    }
}

fn shows_title_bar() -> bool {
    true
}

pub fn load() -> RemoveAppConfig {
    qol_config::load_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_defaults_match_the_typed_defaults() {
        qol_config::validate_contract_defaults_match_type::<RemoveAppConfig>(CONFIG_CONTRACT)
            .unwrap();
        let defaults: RemoveAppConfig = qol_config::typed_defaults_from_contract(CONFIG_CONTRACT)
            .expect("contract defaults must parse");
        assert_eq!(defaults, RemoveAppConfig::default());
    }
}
