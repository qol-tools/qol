use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use super::super::types::{AppState, MAX_CONFIG_SIZE};
use super::http_json::{self, blocking};

type HttpResult<T> = Result<T, Box<Response>>;

#[derive(Serialize)]
struct CoreQueryResponse {
    value: String,
}

pub(in super::super) async fn get_core_query(Path(query): Path<String>) -> impl IntoResponse {
    blocking("core query", move || get_core_query_inner(&query)).await
}

fn get_core_query_inner(query: &str) -> HttpResult<Response> {
    if query == "profiles" {
        let names = crate::features::profile::registry::list_profiles().unwrap_or_default();
        let json = http_json::encode_json(&names, "Failed to serialize profile list")?;
        return Ok(http_json::json_response(json));
    }
    let value = match query {
        "theme" => crate::features::theme::current_theme_key(),
        "accent" => crate::features::theme::current_accent_key(),
        _ => return Err(Box::new(not_found(&format!("unknown query: {query}")))),
    };
    let body = CoreQueryResponse { value };
    let json = http_json::encode_json(&body, "Failed to serialize core query")?;
    Ok(http_json::json_response(json))
}

fn not_found(message: &str) -> Response {
    (StatusCode::NOT_FOUND, message.to_string()).into_response()
}

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

#[derive(Deserialize)]
struct NativeThemeRequest {
    key: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct NativeThemeResponse {
    key: String,
    #[serde(rename = "selectedKey")]
    selected_key: Option<String>,
    keys: Vec<String>,
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

pub(in super::super) async fn get_native_theme() -> impl IntoResponse {
    blocking("theme", native_theme_response).await
}

pub(in super::super) async fn set_native_theme(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    blocking("theme", move || set_native_theme_inner(state, body)).await
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
        apply_theme_to_running_surfaces(&state);
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

fn set_native_theme_inner(state: AppState, body: axum::body::Bytes) -> HttpResult<Response> {
    let previous_key = crate::features::theme::current_native_theme_key();
    let request: NativeThemeRequest = http_json::parse_json_body(body, MAX_CONFIG_SIZE)?;
    match request.key.as_deref() {
        Some(key) => crate::features::theme::save_selected_native_theme_key(key),
        None => crate::features::theme::clear_selected_native_theme_key(),
    }
    .map_err(|error| Box::new(bad_request(&error.to_string())))?;
    if crate::features::theme::current_native_theme_key() != previous_key {
        apply_theme_to_running_surfaces(&state);
    }
    native_theme_response()
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

fn native_theme_response() -> HttpResult<Response> {
    let body = NativeThemeResponse {
        key: crate::features::theme::current_native_theme_key(),
        selected_key: crate::features::theme::selected_native_theme_key()
            .ok()
            .flatten(),
        keys: qol_theme::NATIVE_THEME_KEYS
            .iter()
            .map(|key| key.to_string())
            .collect(),
    };
    let json = http_json::encode_json(&body, "Failed to serialize native theme")?;
    Ok(http_json::json_response(json))
}

fn bad_request(message: &str) -> Response {
    (StatusCode::BAD_REQUEST, message.to_string()).into_response()
}

pub(super) fn apply_theme_to_running_surfaces(state: &AppState) {
    let native = crate::features::theme::current_native_theme_key();
    let accent = crate::features::theme::current_accent_key();
    if !crate::settings_surface::apply_theme(&native, &accent) {
        crate::settings_surface::stop();
        crate::settings_surface::prewarm();
    }
    let Ok(mut manager) = state.plugin_manager.lock() else {
        log::error!("Failed to lock plugin manager to broadcast the theme change");
        return;
    };
    let reached = manager.broadcast_theme_to_running_gpui_daemons(&native, &accent);
    if !reached.is_empty() {
        log::info!(
            "Broadcast theme to {} GPUI daemon(s) after theme change",
            reached.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{get_core_query_inner, native_theme_response, NativeThemeResponse};

    #[test]
    fn core_query_accepts_only_theme_and_accent() {
        assert!(get_core_query_inner("theme").is_ok());
        assert!(get_core_query_inner("accent").is_ok());
        assert!(get_core_query_inner("unknown").is_err());
    }

    #[tokio::test]
    async fn native_theme_round_trips_selected_key() {
        let _env_lock = crate::test_support::env_lock().lock().await;
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());

        async fn read_body(response: super::Response) -> NativeThemeResponse {
            let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            serde_json::from_slice(&bytes).unwrap()
        }

        let initial = read_body(native_theme_response().unwrap()).await;
        assert_eq!(initial.key, qol_theme::DEFAULT_NATIVE_THEME_KEY);
        assert_eq!(initial.selected_key, None);
        assert_eq!(initial.keys, qol_theme::NATIVE_THEME_KEYS.to_vec());

        crate::features::theme::save_selected_native_theme_key("slate").unwrap();
        let saved = read_body(native_theme_response().unwrap()).await;
        assert_eq!(saved.key, "slate");
        assert_eq!(saved.selected_key.as_deref(), Some("slate"));

        assert!(crate::features::theme::save_selected_native_theme_key("violet").is_err());
    }
}
