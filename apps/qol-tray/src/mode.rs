use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModeFlag {
    Dev,
    #[default]
    Prod,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ModeConfig {
    #[serde(default)]
    pub mode: ModeFlag,
}

impl ModeConfig {
    pub fn load() -> Result<Self> {
        let path = crate::paths::mode_config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let config: Self = serde_json::from_str(&content)?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = crate::paths::mode_config_path()?;
        crate::file_io::write_pretty_json(&path, self)
    }

    pub fn set(mode: ModeFlag) -> Result<()> {
        Self { mode }.save()
    }

    pub fn is_dev(&self) -> bool {
        matches!(self.mode, ModeFlag::Dev)
    }

    pub fn is_prod(&self) -> bool {
        matches!(self.mode, ModeFlag::Prod)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_prod() {
        assert_eq!(ModeConfig::default().mode, ModeFlag::Prod);
        assert!(ModeConfig::default().is_prod());
    }

    #[test]
    fn deserializes_dev_and_prod() {
        let cases = [
            (r#"{"mode":"dev"}"#, ModeFlag::Dev),
            (r#"{"mode":"prod"}"#, ModeFlag::Prod),
            (r#"{}"#, ModeFlag::Prod),
        ];
        for (raw, expected) in cases {
            let parsed: ModeConfig = serde_json::from_str(raw).unwrap();
            assert_eq!(parsed.mode, expected, "raw: {raw}");
        }
    }

    #[test]
    fn load_returns_default_when_missing() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        let loaded = ModeConfig::load().unwrap();
        assert_eq!(loaded.mode, ModeFlag::Prod);
    }

    #[test]
    fn set_persists_and_round_trips() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        for mode in [ModeFlag::Dev, ModeFlag::Prod, ModeFlag::Dev] {
            ModeConfig::set(mode).unwrap();
            let loaded = ModeConfig::load().unwrap();
            assert_eq!(loaded.mode, mode, "after set({mode:?})");
        }
    }
}
