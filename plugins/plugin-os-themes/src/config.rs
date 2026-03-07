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
    let paths = qol_plugin_api::config::plugin_config_paths(PLUGIN_NAMES);
    let config: Config = if paths.iter().any(|p| p.exists()) {
        qol_plugin_api::config::load_plugin_config(PLUGIN_NAMES)
    } else {
        Config::default()
    };
    if let Some(path) = paths.last() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&config) {
            Ok(json) => { let _ = std::fs::write(path, json); }
            Err(e) => eprintln!("[shake-to-grow] failed to write config: {e}"),
        }
    }
    eprintln!(
        "[shake-to-grow] config: velocity={} shakiness={} regrow_velocity={} regrow_shakiness={} post_trigger={} scale={} calm_ms={} steps={}",
        config.velocity_threshold, config.shakiness_threshold, config.regrow_velocity_threshold, config.regrow_shakiness_threshold, config.post_trigger_threshold, config.scale_factor, config.calm_duration_ms, config.restore_steps
    );
    config
}
