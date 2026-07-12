use serde::{Deserialize, Serialize};

const CONFIG_CONTRACT: &str = qol_config::plugin_config_contract!();
pub(crate) const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub shake_strictness: f64,
    pub shake_min_extent_px: u32,
    pub shake_window_ms: u64,
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
        "[shake-to-grow] config: strictness={} min_extent={}px window_ms={} scale={} calm_ms={} steps={}",
        config.shake_strictness,
        config.shake_min_extent_px,
        config.shake_window_ms,
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
        assert_eq!(defaults.shake_strictness, 6.5);
        assert_eq!(defaults.shake_min_extent_px, 150);
        assert_eq!(defaults.shake_window_ms, 1000);
        assert_eq!(defaults.restore_steps, 18);
    }
}
