mod search_paths;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DevConfig {
    #[serde(default)]
    pub search_paths: Vec<PathBuf>,
}

impl DevConfig {
    pub fn load() -> Result<Self> {
        let path = crate::paths::dev_config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let config: Self = serde_json::from_str(&content)?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = crate::paths::dev_config_path()?;
        crate::file_io::write_pretty_json(&path, self)
    }

    pub fn effective_search_paths(&self) -> Vec<PathBuf> {
        search_paths::effective_search_paths(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_when_search_paths_absent() {
        let raw = r#"{}"#;
        let config: DevConfig = serde_json::from_str(raw).unwrap();
        assert!(config.search_paths.is_empty());
    }

    #[test]
    fn ignores_a_retired_field_left_in_a_stored_config() {
        let raw = r#"{"search_paths":[],"tooling_gh_account":"KMRH47"}"#;
        let config: DevConfig = serde_json::from_str(raw).unwrap();
        assert!(config.search_paths.is_empty());
    }

    #[test]
    fn round_trips_through_serde_preserving_search_paths() {
        let original = DevConfig {
            search_paths: vec![PathBuf::from("/tmp/foo")],
        };
        let raw = serde_json::to_string(&original).unwrap();
        let parsed: DevConfig = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn save_then_load_preserves_search_paths() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        let initial = DevConfig {
            search_paths: vec![PathBuf::from("/tmp/x"), PathBuf::from("/tmp/y")],
        };
        initial.save().unwrap();

        let loaded = DevConfig::load().unwrap();
        assert_eq!(loaded.search_paths, initial.search_paths);
    }
}
