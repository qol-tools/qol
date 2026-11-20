use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Config {
    pub general: General,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct General {
    pub enabled: bool,
    pub display_name: String,
}

const CONTRACT: &str = qol_config::plugin_config_contract!();

pub fn load() -> Config {
    qol_config::load_plugin_config_from_env_with_contract(env!("QOL_PLUGIN_ID"), CONTRACT)
}

impl Config {
    pub fn greeting(&self) -> String {
        if self.general.enabled {
            return format!("Hello from {}", self.general.display_name);
        }
        format!("{} is turned off", self.general.display_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Config {
        let defaults = qol_config::defaults_json_from_contract(CONTRACT).unwrap();
        serde_json::from_value(defaults).unwrap()
    }

    #[test]
    fn config_type_matches_the_contract_defaults() {
        qol_config::validate_contract_defaults_match_type::<Config>(CONTRACT).unwrap();
    }

    #[test]
    fn the_greeting_follows_the_enabled_and_display_name_settings() {
        let mut config = defaults();
        assert_eq!(config.greeting(), "Hello from My Plugin");

        config.general.display_name = "Desk Lamp".to_string();
        assert_eq!(config.greeting(), "Hello from Desk Lamp");

        config.general.enabled = false;
        assert_eq!(config.greeting(), "Desk Lamp is turned off");
    }
}
