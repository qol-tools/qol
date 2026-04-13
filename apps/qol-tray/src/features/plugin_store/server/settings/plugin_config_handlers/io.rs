use axum::{http::StatusCode, response::IntoResponse, response::Response};

use crate::plugins::PluginConfigManager;

use super::super::super::types::MAX_CONFIG_SIZE;
use super::super::http_json;

pub(super) fn load_plugin_config(plugin_id: &str) -> Result<serde_json::Value, Box<Response>> {
    let config = PluginConfigManager::new()
        .and_then(|manager| manager.get_config(plugin_id))
        .map_err(|_| Box::new(read_config_failed_response()))?;
    Ok(config.unwrap_or_else(empty_config))
}

pub(super) fn parse_config_body(
    body: axum::body::Bytes,
) -> Result<serde_json::Value, Box<Response>> {
    http_json::parse_json_body(body, MAX_CONFIG_SIZE)
}

// Merge-save: reads existing config from disk, overlays the frontend's values on top.
// The frontend only sends fields it manages (those declared in qol-config.toml).
// Daemon-owned fields (devices, backend settings) are preserved from the existing file.
// Without this merge, every color wheel drag would wipe the paired device list.
pub(super) fn save_plugin_config(
    plugin_id: &str,
    config: serde_json::Value,
) -> Result<(), Box<Response>> {
    let manager =
        PluginConfigManager::new().map_err(|_| Box::new(save_config_failed_response()))?;
    let existing = manager
        .get_config(plugin_id)
        .ok()
        .flatten()
        .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
    let merged = merge_config(existing, config);
    manager
        .set_config(plugin_id, merged)
        .map_err(|_| Box::new(save_config_failed_response()))
}

fn merge_config(mut base: serde_json::Value, overlay: serde_json::Value) -> serde_json::Value {
    if let (Some(base_obj), Some(overlay_obj)) = (base.as_object_mut(), overlay.as_object()) {
        for (key, value) in overlay_obj {
            base_obj.insert(key.clone(), value.clone());
        }
        base
    } else {
        overlay
    }
}

pub(super) fn encode_config_json(config: &serde_json::Value) -> Result<Vec<u8>, Box<Response>> {
    http_json::encode_json(config, "Failed to serialize config")
}

fn read_config_failed_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read config").into_response()
}

fn save_config_failed_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save config").into_response()
}

fn empty_config() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

#[cfg(test)]
mod tests {
    use super::load_plugin_config;
    use serde_json::json;
    use std::ffi::OsString;
    use tempfile::TempDir;

    struct ConfigEnvGuard {
        home: Option<OsString>,
        xdg_config_home: Option<OsString>,
        xdg_data_home: Option<OsString>,
    }

    impl ConfigEnvGuard {
        fn new(root: &std::path::Path) -> Self {
            let home = std::env::var_os("HOME");
            let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
            let xdg_data_home = std::env::var_os("XDG_DATA_HOME");
            let home_dir = root.join("home");
            let xdg_config_dir = root.join("xdg-config");
            let xdg_data_dir = root.join("xdg-data");
            std::fs::create_dir_all(&home_dir).unwrap();
            std::fs::create_dir_all(&xdg_config_dir).unwrap();
            std::fs::create_dir_all(&xdg_data_dir).unwrap();
            std::env::set_var("HOME", &home_dir);
            std::env::set_var("XDG_CONFIG_HOME", &xdg_config_dir);
            std::env::set_var("XDG_DATA_HOME", &xdg_data_dir);
            Self {
                home,
                xdg_config_home,
                xdg_data_home,
            }
        }
    }

    impl Drop for ConfigEnvGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.home {
                std::env::set_var("HOME", value);
            }
            if self.home.is_none() {
                std::env::remove_var("HOME");
            }
            if let Some(value) = &self.xdg_config_home {
                std::env::set_var("XDG_CONFIG_HOME", value);
            }
            if self.xdg_config_home.is_none() {
                std::env::remove_var("XDG_CONFIG_HOME");
            }
            if let Some(value) = &self.xdg_data_home {
                std::env::set_var("XDG_DATA_HOME", value);
            }
            if self.xdg_data_home.is_none() {
                std::env::remove_var("XDG_DATA_HOME");
            }
        }
    }

    fn setup_env() -> (
        tokio::sync::MutexGuard<'static, ()>,
        TempDir,
        ConfigEnvGuard,
    ) {
        let guard = crate::test_support::env_lock().blocking_lock();
        let root = TempDir::new().unwrap();
        let env = ConfigEnvGuard::new(root.path());
        (guard, root, env)
    }

    #[test]
    fn missing_plugin_config_returns_empty_object() {
        let (_guard, _root, _env) = setup_env();

        let config = load_plugin_config("plugin-test").unwrap();

        assert_eq!(config, json!({}));
    }
}
