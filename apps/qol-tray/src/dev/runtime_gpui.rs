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
}
