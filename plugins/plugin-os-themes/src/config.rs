use serde::{Deserialize, Serialize};

const PLUGIN_NAMES: &[&str] = &["plugin-os-themes"];

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
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

impl Default for Config {
    fn default() -> Self {
        Self {
            velocity_threshold: 4500.0,
            shakiness_threshold: 75.0,
            regrow_velocity_threshold: 1500.0,
            regrow_shakiness_threshold: 3.0,
            post_trigger_threshold: 1000.0,
            scale_factor: 4,
            calm_duration_ms: 650,
            restore_steps: 18,
        }
    }
}

pub fn load() -> Config {
    let config = load_from_disk();
    persist(&config);
    log_config(&config);
    config
}

fn load_from_disk() -> Config {
    if config_paths().iter().any(|path| path.exists()) {
        return qol_plugin_api::config::load_plugin_config(PLUGIN_NAMES);
    }
    Config::default()
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
    qol_plugin_api::config::plugin_config_paths(PLUGIN_NAMES)
}
