use std::collections::HashMap;
use std::path::Path;

use crate::adapters::BuildFingerprintStore;

use super::types::{BuildFingerprintState, DEV_BUILD_STATE_FILE};

pub(crate) struct JsonBuildFingerprintStore;

impl BuildFingerprintStore for JsonBuildFingerprintStore {
    fn load(&self, config_dir: &Path) -> HashMap<String, String> {
        load_build_fingerprints(config_dir)
    }

    fn save(
        &self,
        config_dir: &Path,
        fingerprints: &HashMap<String, String>,
    ) -> Result<(), String> {
        save_build_fingerprints(config_dir, fingerprints)
    }
}

pub(crate) static JSON_BUILD_FINGERPRINT_STORE: JsonBuildFingerprintStore =
    JsonBuildFingerprintStore;

pub fn load_build_fingerprints(config_dir: &Path) -> HashMap<String, String> {
    let state_path = config_dir.join(DEV_BUILD_STATE_FILE);
    let Ok(content) = std::fs::read_to_string(&state_path) else {
        return HashMap::new();
    };
    serde_json::from_str::<BuildFingerprintState>(&content)
        .map(|state| state.fingerprints)
        .unwrap_or_default()
}

pub fn save_build_fingerprints(
    config_dir: &Path,
    fingerprints: &HashMap<String, String>,
) -> Result<(), String> {
    let state_path = config_dir.join(DEV_BUILD_STATE_FILE);
    let state = BuildFingerprintState {
        fingerprints: fingerprints.clone(),
    };
    let content = serde_json::to_string_pretty(&state)
        .map_err(|e| format!("Failed to serialize build fingerprints: {}", e))?;
    qol_fs::atomic_write(&state_path, content.as_bytes())
        .map_err(|e| format!("Failed to save build fingerprints: {}", e))
}
