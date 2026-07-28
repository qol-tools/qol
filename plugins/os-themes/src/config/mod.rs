use serde::{Deserialize, Serialize};

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();
pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");
pub(crate) type ConfigInspection = qol_config::PluginConfigInspection<Config>;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub enabled: bool,
    pub shake_strictness: f64,
    pub regrow_strictness: f64,
    pub shake_min_extent_px: u32,
    pub regrow_min_extent_px: u32,
    pub shake_window_ms: u64,
    pub scale_factor: u32,
    pub calm_duration_ms: u64,
    pub grow_ms: u32,
    pub shrink_ms: u32,
}

pub fn load() -> Config {
    let config = load_from_disk();
    log_config(&config);
    config
}

pub(crate) fn inspect() -> Result<ConfigInspection, qol_config::PluginConfigInspectionError> {
    qol_config::inspect_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

fn load_from_disk() -> Config {
    qol_config::load_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

#[cfg(test)]
fn contract_defaults() -> Config {
    qol_config::typed_defaults_from_contract(CONFIG_CONTRACT).expect("contract defaults must parse")
}

fn log_config(config: &Config) {
    eprintln!(
        "[shake-to-grow] config: enabled={} strictness={} regrow={} min_extent={}px regrow_extent={}px window_ms={} scale={} calm_ms={} grow_ms={} shrink_ms={}",
        config.enabled,
        config.shake_strictness,
        config.regrow_strictness,
        config.shake_min_extent_px,
        config.regrow_min_extent_px,
        config.shake_window_ms,
        config.scale_factor,
        config.calm_duration_ms,
        config.grow_ms,
        config.shrink_ms,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_defaults_match_runtime_type() {
        qol_config::validate_contract_defaults_match_type::<Config>(CONFIG_CONTRACT).unwrap();

        let defaults = contract_defaults();
        assert!(defaults.enabled, "shake-to-grow defaults to enabled");
        assert_eq!(defaults.shake_strictness, 6.5);
        assert_eq!(defaults.regrow_strictness, 2.5);
        assert_eq!(defaults.shake_min_extent_px, 150);
        assert_eq!(defaults.regrow_min_extent_px, 60);
        assert_eq!(defaults.shake_window_ms, 1000);
        assert_eq!(defaults.calm_duration_ms, 100);
        assert_eq!(defaults.grow_ms, 250);
        assert_eq!(defaults.shrink_ms, 225);
    }
}
