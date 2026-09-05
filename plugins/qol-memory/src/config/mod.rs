use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
pub struct Config {
    pub verify_answers: bool,
    pub verifier_endpoint: String,
}

const CONTRACT: &str = qol_config::plugin_config_contract!();

pub fn load() -> Config {
    qol_config::load_plugin_config_from_env_with_contract(env!("QOL_PLUGIN_ID"), CONTRACT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_settings_match_the_config_contract() {
        qol_config::validate_contract_defaults_match_type::<Config>(CONTRACT).unwrap();
    }
}
