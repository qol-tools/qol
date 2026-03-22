use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::hotkeys::{get_registration_errors, trigger_reload, HotkeyConfig, HotkeyManager};

use super::super::types::MAX_CONFIG_SIZE;
use super::http_json;

type HttpResult<T> = Result<T, Box<Response>>;

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
    HotkeyManager::new().map_err(|_| Box::new(load_failed_response()))
}

fn parse_hotkeys(body: axum::body::Bytes) -> HttpResult<HotkeyConfig> {
    http_json::parse_json_body(body, MAX_CONFIG_SIZE)
}

fn hotkeys_json_response(config: &HotkeyConfig) -> Response {
    let Ok(json) = encode_hotkeys_json(config) else {
        return serialize_failed_response();
    };
    http_json::json_response(json)
}

fn encode_hotkeys_json(config: &HotkeyConfig) -> HttpResult<Vec<u8>> {
    http_json::encode_json(config, "Failed to serialize hotkeys")
}

fn hotkeys_saved_response() -> Response {
    (StatusCode::OK, "Hotkeys saved").into_response()
}

fn load_failed_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to load hotkeys").into_response()
}

fn save_failed_response() -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save hotkeys").into_response()
}

pub(in super::super) async fn get_hotkey_errors() -> impl IntoResponse {
    axum::Json(get_registration_errors())
}

fn serialize_failed_response() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to serialize hotkeys",
    )
        .into_response()
}
