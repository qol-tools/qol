use serde::{Deserialize, Serialize};

const PLUGIN_NAMES: &[&str] = &["plugin-os-themes"];

#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub velocity_threshold: f64,
    pub shakiness_threshold: f64,
    pub post_trigger_threshold: f64,
    pub scale_factor: u32,
    pub calm_duration_ms: u64,
    pub restore_steps: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            velocity_threshold: 3000.0,
            shakiness_threshold: 3.0,
            post_trigger_threshold: 800.0,
            scale_factor: 2,
            calm_duration_ms: 600,
            restore_steps: 8,
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
        "[shake-to-grow] config: velocity_threshold={} shakiness_threshold={} post_trigger_threshold={} scale_factor={} calm_duration_ms={} restore_steps={}",
        config.velocity_threshold, config.shakiness_threshold, config.post_trigger_threshold, config.scale_factor, config.calm_duration_ms, config.restore_steps
    );
    config
}
