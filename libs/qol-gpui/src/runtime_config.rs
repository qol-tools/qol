use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const RUNTIME_SUBDIR: &str = "runtime";
const GPUI_FILE: &str = "gpui.json";

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct GpuiRuntimeConfig {
    #[serde(default)]
    pub ghost_opacity: Option<f32>,
    #[serde(default)]
    pub ghost_debug_color: Option<String>,
}

pub fn gpui_runtime_config_paths() -> Vec<PathBuf> {
    qol_config::config_roots()
        .into_iter()
        .map(|root| root.join(RUNTIME_SUBDIR).join(GPUI_FILE))
        .collect()
}

pub fn load_gpui_runtime_config() -> GpuiRuntimeConfig {
    for path in gpui_runtime_config_paths() {
        let contents = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        match serde_json::from_str::<GpuiRuntimeConfig>(&contents) {
            Ok(cfg) => return cfg,
            Err(e) => {
                eprintln!(
                    "[qol-gpui/runtime_config] failed to parse {}: {}",
                    path.display(),
                    e
                );
            }
        }
    }
    GpuiRuntimeConfig::default()
}
