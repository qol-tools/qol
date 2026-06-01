use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::shortcuts::{executor, store, validation};

use super::super::types::{AppState, MAX_CONFIG_SIZE};
use super::http_json;

type HttpResult<T> = Result<T, Box<Response>>;

pub(in super::super) async fn list_shortcuts() -> impl IntoResponse {
    blocking(list_inner).await
}

pub(in super::super) async fn create_shortcut(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    blocking(move || create_inner(&state, body)).await
}

pub(in super::super) async fn update_shortcut(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    blocking(move || update_inner(&state, &id, body)).await
}

pub(in super::super) async fn delete_shortcut(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    blocking(move || delete_inner(&state, &id)).await
}

pub(in super::super) async fn run_shortcut(Path(id): Path<String>) -> impl IntoResponse {
    blocking(move || run_inner(&id)).await
}

async fn blocking<F>(work: F) -> Response
where
    F: FnOnce() -> HttpResult<Response> + Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(Ok(response)) => response,
        Ok(Err(boxed)) => *boxed,
        Err(error) => {
            log::error!("shortcut handler join error: {}", error);
            server_error("shortcut handler crashed")
        }
    }
}

fn list_inner() -> HttpResult<Response> {
    let config = store::load().map_err(|_| Box::new(load_failed()))?;
    let json = http_json::encode_json(&config, "Failed to serialize shortcuts")?;
    Ok(http_json::json_response(json))
}

fn create_inner(state: &AppState, body: axum::body::Bytes) -> HttpResult<Response> {
    let shortcut = http_json::parse_json_body(body, MAX_CONFIG_SIZE)?;
    let mut config = store::load().map_err(|_| Box::new(load_failed()))?;
    store::add(&mut config, shortcut).map_err(|e| Box::new(bad_request(&e)))?;
    store::save(&config).map_err(|_| Box::new(save_failed()))?;
    trigger_launcher_sync(state);
    let json = http_json::encode_json(&config, "Failed to serialize shortcuts")?;
    Ok(http_json::json_response(json))
}

fn update_inner(state: &AppState, id: &str, body: axum::body::Bytes) -> HttpResult<Response> {
    validation::validate_id(id).map_err(|e| Box::new(bad_request(&e)))?;
    let shortcut: crate::shortcuts::model::Shortcut =
        http_json::parse_json_body(body, MAX_CONFIG_SIZE)?;
    if shortcut.id != id {
        return Err(Box::new(bad_request("body id must match path id")));
    }
    let mut config = store::load().map_err(|_| Box::new(load_failed()))?;
    store::update(&mut config, shortcut).map_err(|e| Box::new(bad_request(&e)))?;
    store::save(&config).map_err(|_| Box::new(save_failed()))?;
    trigger_launcher_sync(state);
    let json = http_json::encode_json(&config, "Failed to serialize shortcuts")?;
    Ok(http_json::json_response(json))
}

fn delete_inner(state: &AppState, id: &str) -> HttpResult<Response> {
    validation::validate_id(id).map_err(|e| Box::new(bad_request(&e)))?;
    let mut config = store::load().map_err(|_| Box::new(load_failed()))?;
    store::remove(&mut config, id).map_err(|e| Box::new(not_found(&e)))?;
    store::save(&config).map_err(|_| Box::new(save_failed()))?;
    trigger_launcher_sync(state);
    let json = http_json::encode_json(&config, "Failed to serialize shortcuts")?;
    Ok(http_json::json_response(json))
}

fn run_inner(id: &str) -> HttpResult<Response> {
    validation::validate_id(id).map_err(|e| Box::new(bad_request(&e)))?;
    let config = store::load().map_err(|_| Box::new(load_failed()))?;
    let shortcut = store::find_by_id(&config, id)
        .ok_or_else(|| Box::new(not_found(&format!("shortcut '{}' not found", id))))?;
    executor::execute(&shortcut).map_err(|e| Box::new(server_error(&e.to_string())))?;
    Ok((StatusCode::OK, "Shortcut executed").into_response())
}

fn trigger_launcher_sync(_state: &AppState) {
    crate::features::launcher_apps::trigger_full_sync();
}

fn load_failed() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to load shortcuts",
    )
        .into_response()
}

fn save_failed() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Failed to save shortcuts",
    )
        .into_response()
}

fn bad_request(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, msg.to_string()).into_response()
}

fn not_found(msg: &str) -> Response {
    (StatusCode::NOT_FOUND, msg.to_string()).into_response()
}

fn server_error(msg: &str) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, msg.to_string()).into_response()
}
