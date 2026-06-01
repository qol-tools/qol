use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::hotkeys::{get_registration_errors, trigger_reload, HotkeyConfig, HotkeyManager};

use super::super::types::MAX_CONFIG_SIZE;
use super::http_json;

type HttpResult<T> = Result<T, Box<Response>>;

pub(in super::super) async fn get_hotkeys() -> impl IntoResponse {
    blocking(get_hotkeys_inner).await
}

pub(in super::super) async fn set_hotkeys(body: axum::body::Bytes) -> impl IntoResponse {
    blocking(move || set_hotkeys_inner(body)).await
}

pub(in super::super) async fn open_hotkeys_file() -> impl IntoResponse {
    let path = crate::paths::hotkeys_path();
    blocking_open(move || open_config_file(path)).await
}

pub(in super::super) async fn open_shortcuts_file() -> impl IntoResponse {
    let path = crate::paths::shortcuts_path();
    blocking_open(move || open_config_file(path)).await
}

async fn blocking<F>(work: F) -> Response
where
    F: FnOnce() -> HttpResult<Response> + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(Ok(response)) => response,
        Ok(Err(boxed)) => *boxed,
        Err(error) => {
            log::error!("hotkey handler join error: {}", error);
            (StatusCode::INTERNAL_SERVER_ERROR, "Handler crashed").into_response()
        }
    }
}

async fn blocking_open<F>(work: F) -> Response
where
    F: FnOnce() -> Response + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(response) => response,
        Err(error) => {
            log::error!("hotkey open handler join error: {}", error);
            (StatusCode::INTERNAL_SERVER_ERROR, "Handler crashed").into_response()
        }
    }
}

fn open_config_file(path: anyhow::Result<std::path::PathBuf>) -> Response {
    let Ok(path) = path else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "Path unavailable").into_response();
    };
    match crate::features::profile::sync::platform::open_path(&path) {
        Ok(()) => StatusCode::OK.into_response(),
        Err(error) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{error}")).into_response(),
    }
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
