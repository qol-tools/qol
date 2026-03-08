use axum::{http::StatusCode, response::IntoResponse, response::Response};

use crate::plugins::PluginConfigManager;

use super::super::super::types::MAX_CONFIG_SIZE;
use super::super::http_json;

pub(super) fn load_plugin_config(plugin_id: &str) -> Result<serde_json::Value, Box<Response>> {
    let config = PluginConfigManager::new()
        .and_then(|manager| manager.get_config(plugin_id))
        .map_err(|_| Box::new(read_config_failed_response()))?;
    config.ok_or_else(|| Box::new(config_not_found_response()))
}

pub(super) fn parse_config_body(
    body: axum::body::Bytes,
) -> Result<serde_json::Value, Box<Response>> {
    http_json::parse_json_body(body, MAX_CONFIG_SIZE)
}

pub(super) fn save_plugin_config(
    plugin_id: &str,
    config: serde_json::Value,
) -> Result<(), Box<Response>> {
    PluginConfigManager::new()
        .and_then(|manager| manager.set_config(plugin_id, config))
        .map_err(|_| Box::new(save_config_failed_response()))
}

pub(super) fn encode_config_json(config: &serde_json::Value) -> Result<Vec<u8>, Box<Response>> {
    http_json::encode_json(config, "Failed to serialize config")
}

fn config_not_found_response() -> Response {
    (StatusCode::NOT_FOUND, "Config not found").into_response()
}

fn read_config_failed_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read config").into_response()
}

fn save_config_failed_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save config").into_response()
}
