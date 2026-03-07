use serde::{Deserialize, Serialize};

const PLUGIN_NAMES: &[&str] = &["plugin-os-themes"];

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub velocity_threshold: f64,
    pub scale_factor: u32,
    pub calm_duration_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            velocity_threshold: 3000.0,
            scale_factor: 2,
            calm_duration_ms: 600,
        }
    }
}

pub fn load() -> Config {
    let config: Config = qol_plugin_api::config::load_plugin_config(PLUGIN_NAMES);
    eprintln!(
        "[shake-to-grow] config: velocity_threshold={} scale_factor={} calm_duration_ms={}",
        config.velocity_threshold, config.scale_factor, config.calm_duration_ms
    );
    config
}
