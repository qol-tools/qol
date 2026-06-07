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

    pub fn set_ghost_debug_color(value: Option<&str>) -> Result<()> {
        let mut config = Self::load().unwrap_or_default();
        config.ghost_debug_color = normalize_color(value);
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

pub fn normalize_color(value: Option<&str>) -> Option<String> {
    let raw = value?.trim();
    if raw.is_empty() {
        return None;
    }
    let body = raw.strip_prefix('#').unwrap_or(raw);
    if body.len() != 6 {
        return None;
    }
    if !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("#{}", body.to_ascii_lowercase()))
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
    fn normalize_color_table() {
        let cases: &[(Option<&str>, Option<&str>)] = &[
            (None, None),
            (Some(""), None),
            (Some("   "), None),
            (Some("#ff8800"), Some("#ff8800")),
            (Some("ff8800"), Some("#ff8800")),
            (Some("  #FF8800  "), Some("#ff8800")),
            (Some("#FFAACC"), Some("#ffaacc")),
            (Some("#fff"), None),
            (Some("#ff88000"), None),
            (Some("#zzzzzz"), None),
            (Some("not-a-color"), None),
            (Some("#12 3456"), None),
        ];
        for (input, expected) in cases {
            assert_eq!(
                normalize_color(*input).as_deref(),
                *expected,
                "input: {:?}",
                input
            );
        }
    }

    #[test]
    fn set_ghost_debug_color_writes_and_reads_back() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        GpuiRuntimeConfig::set_ghost_debug_color(Some("FF8800")).unwrap();
        let loaded = GpuiRuntimeConfig::load().unwrap();
        assert_eq!(loaded.ghost_debug_color.as_deref(), Some("#ff8800"));
    }

    #[test]
    fn set_ghost_debug_color_rejects_invalid_hex_by_clearing() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        GpuiRuntimeConfig::set_ghost_debug_color(Some("#ff8800")).unwrap();
        GpuiRuntimeConfig::set_ghost_debug_color(Some("not-a-color")).unwrap();
        let loaded = GpuiRuntimeConfig::load().unwrap();
        assert_eq!(loaded.ghost_debug_color, None);
    }

    #[test]
    fn set_ghost_debug_color_clears_with_none() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        GpuiRuntimeConfig::set_ghost_debug_color(Some("#abcdef")).unwrap();
        GpuiRuntimeConfig::set_ghost_debug_color(None).unwrap();
        let loaded = GpuiRuntimeConfig::load().unwrap();
        assert_eq!(loaded.ghost_debug_color, None);
    }

    #[test]
    fn opacity_and_color_persist_independently() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());

        GpuiRuntimeConfig::set_ghost_opacity(Some(0.3)).unwrap();
        GpuiRuntimeConfig::set_ghost_debug_color(Some("#112233")).unwrap();
        let after_color = GpuiRuntimeConfig::load().unwrap();
        assert_eq!(after_color.ghost_opacity, Some(0.3));
        assert_eq!(after_color.ghost_debug_color.as_deref(), Some("#112233"));

        GpuiRuntimeConfig::set_ghost_opacity(None).unwrap();
        let after_clear = GpuiRuntimeConfig::load().unwrap();
        assert_eq!(after_clear.ghost_opacity, None);
        assert_eq!(after_clear.ghost_debug_color.as_deref(), Some("#112233"));
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
