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

    pub fn set_ghost_opacity(value: Option<f32>) -> Result<()> {
        let mut config = Self::load().unwrap_or_default();
        config.ghost_opacity = clamp_opacity(value);
        config.save()
    }
}

fn clamp_opacity(value: Option<f32>) -> Option<f32> {
    let raw = value?;
    if !raw.is_finite() {
        return None;
    }
    Some(raw.clamp(0.0, 1.0))
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
    fn clamp_opacity_table() {
        let cases: &[(Option<f32>, Option<f32>)] = &[
            (None, None),
            (Some(0.0), Some(0.0)),
            (Some(0.5), Some(0.5)),
            (Some(1.0), Some(1.0)),
            (Some(-0.1), Some(0.0)),
            (Some(1.5), Some(1.0)),
            (Some(f32::NAN), None),
            (Some(f32::INFINITY), None),
            (Some(f32::NEG_INFINITY), None),
        ];
        for (input, expected) in cases {
            assert_eq!(clamp_opacity(*input), *expected, "input: {:?}", input);
        }
    }

    #[test]
    fn set_ghost_opacity_writes_and_reads_back() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        GpuiRuntimeConfig::set_ghost_opacity(Some(0.42)).unwrap();
        let loaded = GpuiRuntimeConfig::load().unwrap();
        assert_eq!(loaded.ghost_opacity, Some(0.42));
    }

    #[test]
    fn set_ghost_opacity_clamps_out_of_range() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        GpuiRuntimeConfig::set_ghost_opacity(Some(2.0)).unwrap();
        let loaded = GpuiRuntimeConfig::load().unwrap();
        assert_eq!(loaded.ghost_opacity, Some(1.0));
    }

    #[test]
    fn load_returns_default_when_missing() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        let loaded = GpuiRuntimeConfig::load().unwrap();
        assert_eq!(loaded, GpuiRuntimeConfig::default());
    }

    #[test]
    fn set_ghost_opacity_clears_with_none() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        GpuiRuntimeConfig::set_ghost_opacity(Some(0.5)).unwrap();
        GpuiRuntimeConfig::set_ghost_opacity(None).unwrap();
        let loaded = GpuiRuntimeConfig::load().unwrap();
        assert_eq!(loaded.ghost_opacity, None);
    }
}
