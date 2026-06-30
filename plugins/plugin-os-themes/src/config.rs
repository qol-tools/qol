use serde::{Deserialize, Serialize};

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();
pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub velocity_threshold: f64,
    pub shakiness_threshold: f64,
    pub regrow_velocity_threshold: f64,
    pub regrow_shakiness_threshold: f64,
    pub post_trigger_threshold: f64,
    pub scale_factor: u32,
    pub calm_duration_ms: u64,
    pub restore_steps: u32,
}

pub fn load() -> Config {
    let config = load_from_disk();
    persist(&config);
    log_config(&config);
    config
}

fn load_from_disk() -> Config {
    qol_config::load_plugin_config_from_env_with_contract(PLUGIN_ID, CONFIG_CONTRACT)
}

#[cfg(test)]
fn contract_defaults() -> Config {
    qol_config::typed_defaults_from_contract(CONFIG_CONTRACT).expect("contract defaults must parse")
}

fn persist(config: &Config) {
    let Some(path) = config_paths().into_iter().last() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(config) {
        Ok(json) => {
            let _ = std::fs::write(path, json);
        }
        Err(error) => eprintln!("[shake-to-grow] failed to write config: {error}"),
    }
}

fn log_config(config: &Config) {
    eprintln!(
        "[shake-to-grow] config: velocity={} shakiness={} regrow_velocity={} regrow_shakiness={} post_trigger={} scale={} calm_ms={} steps={}",
        config.velocity_threshold,
        config.shakiness_threshold,
        config.regrow_velocity_threshold,
        config.regrow_shakiness_threshold,
        config.post_trigger_threshold,
        config.scale_factor,
        config.calm_duration_ms,
        config.restore_steps,
    );
}

fn config_paths() -> Vec<std::path::PathBuf> {
    qol_config::plugin_config_paths_from_env(PLUGIN_ID)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_defaults_match_runtime_type() {
        qol_config::validate_contract_defaults_match_type::<Config>(CONFIG_CONTRACT).unwrap();

        let defaults = contract_defaults();
        assert_eq!(defaults.velocity_threshold, 4500.0);
        assert_eq!(defaults.restore_steps, 18);
    }
}
