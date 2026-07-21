use serde::{Deserialize, Serialize};

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();

pub fn contract() -> &'static str {
    CONFIG_CONTRACT
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ReconnectConfig {
    pub managed_devices: Vec<String>,
    pub auto_reconnect: bool,
    pub power_on_adapter: bool,
    pub retry_initial_seconds: f64,
    pub retry_max_seconds: f64,
}

pub fn load() -> ReconnectConfig {
    qol_config::load_plugin_config_from_env_with_contract(crate::PLUGIN_ID, CONFIG_CONTRACT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_defaults_match_runtime_type() {
        qol_config::validate_contract_defaults_match_type::<ReconnectConfig>(CONFIG_CONTRACT)
            .unwrap();
        let config: ReconnectConfig =
            qol_config::typed_defaults_from_contract(CONFIG_CONTRACT).unwrap();
        assert!(config.managed_devices.is_empty());
        assert!(config.auto_reconnect);
        assert!(config.power_on_adapter);
        assert_eq!(config.retry_initial_seconds, 1.0);
        assert_eq!(config.retry_max_seconds, 60.0);
    }
}
