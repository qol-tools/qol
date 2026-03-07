use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};

use crate::hotkeys::{trigger_reload, HotkeyConfig, HotkeyManager};

use super::super::types::MAX_CONFIG_SIZE;

type HttpResult<T> = Result<T, Box<Response>>;
const APPLICATION_JSON: &str = "application/json";

pub(in super::super) async fn get_hotkeys() -> impl IntoResponse {
    get_hotkeys_inner().unwrap_or_else(|response| *response)
}

pub(in super::super) async fn set_hotkeys(body: axum::body::Bytes) -> impl IntoResponse {
    set_hotkeys_inner(body).unwrap_or_else(|response| *response)
}

fn get_hotkeys_inner() -> HttpResult<Response> {
    let manager = hotkey_manager()?;
    let config = manager
        .load_config()
        .map_err(|_| Box::new(load_failed_response()))?;
    Ok(hotkeys_json_response(&config))
}

fn set_hotkeys_inner(body: axum::body::Bytes) -> HttpResult<Response> {
    let config = parse_hotkeys(body)?;
    let manager = hotkey_manager()?;
    manager
        .save_config(&config)
        .map_err(|_| Box::new(save_failed_response()))?;
    trigger_reload();
    Ok(hotkeys_saved_response())
}

fn hotkey_manager() -> HttpResult<HotkeyManager> {
    HotkeyManager::new().map_err(|_| Box::new(manager_failed_response()))
}

fn parse_hotkeys(body: axum::body::Bytes) -> HttpResult<HotkeyConfig> {
    if body.len() > MAX_CONFIG_SIZE {
        return Err(Box::new(config_too_large_response()));
    }
    serde_json::from_slice(&body).map_err(|_| Box::new(invalid_json_response()))
}

fn hotkeys_json_response(config: &HotkeyConfig) -> Response {
    let Ok(json) = encode_hotkeys_json(config) else {
        return serialize_failed_response();
    };
    json_response(json)
}

fn encode_hotkeys_json(config: &HotkeyConfig) -> HttpResult<Vec<u8>> {
    serde_json::to_vec(config).map_err(|_| Box::new(serialize_failed_response()))
}

fn json_response(json: Vec<u8>) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, APPLICATION_JSON)],
        json,
    )
        .into_response()
}

fn hotkeys_saved_response() -> Response {
    (StatusCode::OK, "Hotkeys saved").into_response()
}

fn manager_failed_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to load hotkeys").into_response()
}

fn load_failed_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to load hotkeys").into_response()
}

fn save_failed_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save hotkeys").into_response()
}

fn serialize_failed_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to serialize hotkeys",
    )
        .into_response()
}

fn invalid_json_response() -> Response {
    (StatusCode::BAD_REQUEST, "Invalid JSON").into_response()
}

fn config_too_large_response() -> Response {
    (StatusCode::PAYLOAD_TOO_LARGE, "Config too large").into_response()
}
