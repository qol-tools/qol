use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::file_io;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct GpuiRuntimeConfig {
    #[serde(default)]
    pub ghost_opacity: Option<f32>,
    #[serde(default)]
    pub ghost_debug_color: Option<String>,
}

impl GpuiRuntimeConfig {
    pub fn load() -> Result<Self> {
        let path = crate::paths::runtime_gpui_config_path()?;
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = std::fs::read_to_string(&path)?;
        let config: Self = serde_json::from_str(&content)?;
        Ok(config)
    }

    pub fn save(&self) -> Result<()> {
        let path = crate::paths::runtime_gpui_config_path()?;
        file_io::write_pretty_json(&path, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_with_ghost_opacity_present() {
        let raw = r#"{"ghost_opacity":0.5}"#;
        let config: GpuiRuntimeConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(config.ghost_opacity, Some(0.5));
    }

    #[test]
    fn deserializes_when_ghost_opacity_absent() {
        let raw = r#"{}"#;
        let config: GpuiRuntimeConfig = serde_json::from_str(raw).unwrap();
        assert_eq!(config.ghost_opacity, None);
    }

    #[test]
    fn load_returns_default_when_missing() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        let loaded = GpuiRuntimeConfig::load().unwrap();
        assert_eq!(loaded, GpuiRuntimeConfig::default());
    }
}
