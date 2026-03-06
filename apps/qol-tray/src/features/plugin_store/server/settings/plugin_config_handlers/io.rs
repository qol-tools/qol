use axum::{http::StatusCode, response::IntoResponse, response::Response};

use crate::plugins::PluginConfigManager;

use super::super::super::types::MAX_CONFIG_SIZE;

pub(super) fn load_plugin_config(plugin_id: &str) -> Result<serde_json::Value, Response> {
    let config = PluginConfigManager::new()
        .and_then(|manager| manager.get_config(plugin_id))
        .map_err(|_| read_config_failed_response())?;
    config.ok_or_else(config_not_found_response)
}

pub(super) fn parse_config_body(body: axum::body::Bytes) -> Result<serde_json::Value, Response> {
    if body.len() > MAX_CONFIG_SIZE {
        return Err(config_too_large_response());
    }
    serde_json::from_slice(&body).map_err(|_| invalid_json_response())
}

pub(super) fn save_plugin_config(
    plugin_id: &str,
    config: serde_json::Value,
) -> Result<(), Response> {
    PluginConfigManager::new()
        .and_then(|manager| manager.set_config(plugin_id, config))
        .map_err(|_| save_config_failed_response())
}

pub(super) fn encode_config_json(config: &serde_json::Value) -> Result<Vec<u8>, Response> {
    serde_json::to_vec(config).map_err(|_| serialize_config_failed_response())
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

fn serialize_config_failed_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to serialize config",
    )
        .into_response()
}

fn invalid_json_response() -> Response {
    (StatusCode::BAD_REQUEST, "Invalid JSON").into_response()
}

fn config_too_large_response() -> Response {
    (StatusCode::PAYLOAD_TOO_LARGE, "Config too large").into_response()
}
