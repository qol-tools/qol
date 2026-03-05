use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use crate::plugins::PluginConfigManager;

use super::super::helpers::validate_plugin_id_bad_request;
use super::super::types::{AppState, MAX_CONFIG_SIZE};

type HttpResult<T> = Result<T, Response>;
const APPLICATION_JSON: &str = "application/json";

pub(in super::super) async fn get_plugin_config(
    Path(plugin_id): Path<String>,
) -> impl IntoResponse {
    get_plugin_config_inner(plugin_id).unwrap_or_else(|response| response)
}

pub(in super::super) async fn set_plugin_config(
    Path(plugin_id): Path<String>,
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    set_plugin_config_inner(plugin_id, &state, body).unwrap_or_else(|response| response)
}

fn get_plugin_config_inner(plugin_id: String) -> HttpResult<Response> {
    let plugin_id = validated_plugin_id(plugin_id)?;
    let config = load_plugin_config(&plugin_id)?;
    Ok(config_json_response(&config))
}

fn set_plugin_config_inner(
    plugin_id: String,
    state: &AppState,
    body: axum::body::Bytes,
) -> HttpResult<Response> {
    let plugin_id = validated_plugin_id(plugin_id)?;
    let config = parse_config_body(body)?;
    save_plugin_config(&plugin_id, config)?;
    notify_plugin_reload(state, &plugin_id);
    Ok(config_saved_response())
}

fn validated_plugin_id(plugin_id: String) -> HttpResult<String> {
    validate_plugin_id_bad_request(&plugin_id).map_err(IntoResponse::into_response)?;
    Ok(plugin_id)
}

fn load_plugin_config(plugin_id: &str) -> HttpResult<serde_json::Value> {
    let config = PluginConfigManager::new()
        .and_then(|manager| manager.get_config(plugin_id))
        .map_err(|_| read_config_failed_response())?;
    config.ok_or_else(config_not_found_response)
}

fn parse_config_body(body: axum::body::Bytes) -> HttpResult<serde_json::Value> {
    if body.len() > MAX_CONFIG_SIZE {
        return Err(config_too_large_response());
    }
    serde_json::from_slice(&body).map_err(|_| invalid_json_response())
}

fn save_plugin_config(plugin_id: &str, config: serde_json::Value) -> HttpResult<()> {
    PluginConfigManager::new()
        .and_then(|manager| manager.set_config(plugin_id, config))
        .map_err(|_| save_config_failed_response())
}

fn config_json_response(config: &serde_json::Value) -> Response {
    let Ok(json) = encode_config_json(config) else {
        return serialize_config_failed_response();
    };
    json_response(json)
}

fn encode_config_json(config: &serde_json::Value) -> HttpResult<Vec<u8>> {
    serde_json::to_vec(config).map_err(|_| serialize_config_failed_response())
}

fn json_response(json: Vec<u8>) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, APPLICATION_JSON)],
        json,
    )
        .into_response()
}

fn config_saved_response() -> Response {
    (StatusCode::OK, "Config saved").into_response()
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

#[cfg(unix)]
fn notify_plugin_reload(state: &AppState, plugin_id: &str) {
    let Some(socket_path) = daemon_socket_path(state, plugin_id) else {
        return;
    };
    send_reload(socket_path);
}

#[cfg(unix)]
fn daemon_socket_path(state: &AppState, plugin_id: &str) -> Option<String> {
    let manager = state.plugin_manager.lock().unwrap();
    manager
        .get(plugin_id)
        .and_then(|plugin| plugin.manifest.daemon.as_ref()?.socket.clone())
}

#[cfg(unix)]
fn send_reload(socket_path: String) {
    use std::io::Write;
    use std::os::unix::net::UnixStream;

    let Ok(mut stream) = UnixStream::connect(&socket_path) else {
        return;
    };
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_millis(500)));
    let _ = stream.write_all(b"reload");
}

#[cfg(not(unix))]
fn notify_plugin_reload(_state: &AppState, _plugin_id: &str) {}
