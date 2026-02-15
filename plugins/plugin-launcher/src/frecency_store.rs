use std::path::{Path, PathBuf};

use crate::frecency::FrequencyData;

pub fn default_path() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("qol-launcher-frequency.json")
}

pub fn load(path: &Path) -> FrequencyData {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok())
        .unwrap_or_default()
}

pub fn save(path: &Path, data: &FrequencyData) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(data) {
        Ok(json) => {
            if let Err(e) = std::fs::write(path, json) {
                eprintln!("[frecency_store] failed to write {}: {}", path.display(), e);
            }
        }
        Err(e) => eprintln!("[frecency_store] failed to serialize: {}", e),
    }
}
