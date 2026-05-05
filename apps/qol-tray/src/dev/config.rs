mod search_paths;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct DevConfig {
    #[serde(default)]
    pub search_paths: Vec<PathBuf>,
    #[serde(default)]
    pub tooling_gh_account: Option<String>,
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

    pub fn set_tooling_gh_account(value: Option<String>) -> Result<()> {
        let mut config = Self::load().unwrap_or_default();
        config.tooling_gh_account = normalize_account(value);
        config.save()
    }

    pub fn effective_search_paths(&self) -> Vec<PathBuf> {
        search_paths::effective_search_paths(self)
    }
}

fn normalize_account(value: Option<String>) -> Option<String> {
    let trimmed = value?.trim().to_string();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_with_tooling_gh_account_present() {
        let raw = r#"{"search_paths":[],"tooling_gh_account":"KMRH47"}"#;
        let config: DevConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(config.tooling_gh_account.as_deref(), Some("KMRH47"));
    }

    #[test]
    fn deserializes_when_tooling_gh_account_absent() {
        let raw = r#"{"search_paths":[]}"#;
        let config: DevConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(config.tooling_gh_account, None);
    }

    #[test]
    fn deserializes_when_tooling_gh_account_explicit_null() {
        let raw = r#"{"search_paths":[],"tooling_gh_account":null}"#;
        let config: DevConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(config.tooling_gh_account, None);
    }

    #[test]
    fn round_trips_through_serde_preserving_search_paths() {
        let original = DevConfig {
            search_paths: vec![PathBuf::from("/tmp/foo")],
            tooling_gh_account: Some("octocat".to_string()),
        };
        let raw = serde_json::to_string(&original).unwrap();
        let parsed: DevConfig = serde_json::from_str(&raw).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn normalize_trims_whitespace() {
        assert_eq!(
            normalize_account(Some("  KMRH47  ".to_string())),
            Some("KMRH47".to_string())
        );
    }

    #[test]
    fn normalize_collapses_empty_to_none() {
        assert_eq!(normalize_account(Some(String::new())), None);
        assert_eq!(normalize_account(Some("   ".to_string())), None);
        assert_eq!(normalize_account(None), None);
    }

    #[test]
    fn set_tooling_gh_account_writes_and_reads_back() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        DevConfig::set_tooling_gh_account(Some("KMRH47".to_string())).unwrap();

        let loaded = DevConfig::load().unwrap();
        assert_eq!(loaded.tooling_gh_account.as_deref(), Some("KMRH47"));
    }

    #[test]
    fn set_tooling_gh_account_preserves_search_paths() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        let initial = DevConfig {
            search_paths: vec![PathBuf::from("/tmp/x"), PathBuf::from("/tmp/y")],
            tooling_gh_account: None,
        };
        initial.save().unwrap();

        DevConfig::set_tooling_gh_account(Some("octocat".to_string())).unwrap();

        let loaded = DevConfig::load().unwrap();
        assert_eq!(loaded.search_paths, initial.search_paths);
        assert_eq!(loaded.tooling_gh_account.as_deref(), Some("octocat"));
    }

    #[test]
    fn set_tooling_gh_account_clears_with_none() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        DevConfig::set_tooling_gh_account(Some("KMRH47".to_string())).unwrap();
        DevConfig::set_tooling_gh_account(None).unwrap();

        let loaded = DevConfig::load().unwrap();
        assert_eq!(loaded.tooling_gh_account, None);
    }

    #[test]
    fn set_tooling_gh_account_is_idempotent() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        DevConfig::set_tooling_gh_account(Some("KMRH47".to_string())).unwrap();
        let first = std::fs::read_to_string(crate::paths::dev_config_path().unwrap()).unwrap();
        DevConfig::set_tooling_gh_account(Some("KMRH47".to_string())).unwrap();
        let second = std::fs::read_to_string(crate::paths::dev_config_path().unwrap()).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn set_tooling_gh_account_normalizes_whitespace() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        DevConfig::set_tooling_gh_account(Some("  KMRH47  ".to_string())).unwrap();
        let loaded = DevConfig::load().unwrap();
        assert_eq!(loaded.tooling_gh_account.as_deref(), Some("KMRH47"));
    }

    #[test]
    fn set_tooling_gh_account_empty_string_is_none() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        DevConfig::set_tooling_gh_account(Some(String::new())).unwrap();
        let loaded = DevConfig::load().unwrap();
        assert_eq!(loaded.tooling_gh_account, None);
    }
}
