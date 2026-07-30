use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use super::super::types::{AppState, MAX_CONFIG_SIZE};
use super::http_json::{self, blocking};

type HttpResult<T> = Result<T, Box<Response>>;

#[derive(Deserialize)]
struct ThemeAccentRequest {
    key: Option<String>,
}

#[derive(Serialize)]
struct ThemeAccentResponse {
    key: String,
    #[serde(rename = "selectedKey")]
    selected_key: Option<String>,
}

#[derive(Deserialize)]
struct ThemeRequest {
    key: Option<String>,
}

#[derive(Serialize)]
struct ThemeResponse {
    key: String,
    #[serde(rename = "selectedKey")]
    selected_key: Option<String>,
}

pub(in super::super) async fn get_theme_accent() -> impl IntoResponse {
    blocking("theme", get_theme_accent_inner).await
}

pub(in super::super) async fn set_theme_accent(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    blocking("theme", move || set_theme_accent_inner(state, body)).await
}

pub(in super::super) async fn get_theme() -> impl IntoResponse {
    blocking("theme", theme_response).await
}

pub(in super::super) async fn set_theme(body: axum::body::Bytes) -> impl IntoResponse {
    blocking("theme", move || set_theme_inner(body)).await
}

fn get_theme_accent_inner() -> HttpResult<Response> {
    theme_accent_response()
}

fn set_theme_accent_inner(state: AppState, body: axum::body::Bytes) -> HttpResult<Response> {
    let previous_key = crate::features::theme::current_accent_key();
    let request: ThemeAccentRequest = http_json::parse_json_body(body, MAX_CONFIG_SIZE)?;
    match request.key.as_deref() {
        Some(key) => crate::features::theme::save_selected_accent_key(key),
        None => crate::features::theme::clear_selected_accent_key(),
    }
    .map_err(|error| Box::new(bad_request(&error.to_string())))?;
    let current_key = crate::features::theme::current_accent_key();
    if current_key != previous_key {
        restart_running_gpui_daemons(&state);
    }
    theme_accent_response()
}

fn set_theme_inner(body: axum::body::Bytes) -> HttpResult<Response> {
    let request: ThemeRequest = http_json::parse_json_body(body, MAX_CONFIG_SIZE)?;
    match request.key.as_deref() {
        Some(key) => crate::features::theme::save_selected_theme_key(key),
        None => crate::features::theme::clear_selected_theme_key(),
    }
    .map_err(|error| Box::new(bad_request(&error.to_string())))?;
    theme_response()
}

fn theme_response() -> HttpResult<Response> {
    let body = ThemeResponse {
        key: crate::features::theme::current_theme_key(),
        selected_key: crate::features::theme::selected_theme_key().ok().flatten(),
    };
    let json = http_json::encode_json(&body, "Failed to serialize theme")?;
    Ok(http_json::json_response(json))
}

fn theme_accent_response() -> HttpResult<Response> {
    let body = ThemeAccentResponse {
        key: crate::features::theme::current_accent_key(),
        selected_key: crate::features::theme::selected_accent_key().ok().flatten(),
    };
    let json = http_json::encode_json(&body, "Failed to serialize theme accent")?;
    Ok(http_json::json_response(json))
}

fn bad_request(message: &str) -> Response {
    (StatusCode::BAD_REQUEST, message.to_string()).into_response()
}

fn restart_running_gpui_daemons(state: &AppState) {
    crate::settings_surface::stop();
    let Ok(mut manager) = state.plugin_manager.lock() else {
        log::error!("Failed to lock plugin manager after theme accent change");
        return;
    };
    let restarted = manager.restart_running_gpui_daemons();
    if !restarted.is_empty() {
        log::info!(
            "Restarted {} GPUI daemon(s) after theme accent change",
            restarted.len()
        );
    }
}
