use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::daemon::ConfigKind;
use crate::shortcuts::{executor, store, validation};

use super::super::types::{AppState, MAX_CONFIG_SIZE};
use super::http_json::{self, blocking};

type HttpResult<T> = Result<T, Box<Response>>;

pub(in super::super) async fn list_shortcuts() -> impl IntoResponse {
    blocking("shortcut", list_inner).await
}

pub(in super::super) async fn create_shortcut(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    blocking("shortcut", move || create_inner(&state, body)).await
}

pub(in super::super) async fn update_shortcut(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    blocking("shortcut", move || update_inner(&state, &id, body)).await
}

pub(in super::super) async fn delete_shortcut(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    blocking("shortcut", move || delete_inner(&state, &id)).await
}

pub(in super::super) async fn run_shortcut(Path(id): Path<String>) -> impl IntoResponse {
    blocking("shortcut", move || run_inner(&id)).await
}

fn list_inner() -> HttpResult<Response> {
    let config = store::load().map_err(|_| Box::new(load_failed()))?;
    let json = http_json::encode_json(&config, "Failed to serialize shortcuts")?;
    Ok(http_json::json_response(json))
}

fn create_inner(state: &AppState, body: axum::body::Bytes) -> HttpResult<Response> {
    let shortcut = http_json::parse_json_body(body, MAX_CONFIG_SIZE)?;
    let config = store::create_persisted(shortcut)
        .map_err(|error| map_mutation_error(error, bad_request))?;
    state.daemon.config.config_changed(ConfigKind::Shortcuts);
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
    let config = store::update_persisted(shortcut)
        .map_err(|error| map_mutation_error(error, bad_request))?;
    state.daemon.config.config_changed(ConfigKind::Shortcuts);
    let json = http_json::encode_json(&config, "Failed to serialize shortcuts")?;
    Ok(http_json::json_response(json))
}

fn delete_inner(state: &AppState, id: &str) -> HttpResult<Response> {
    validation::validate_id(id).map_err(|e| Box::new(bad_request(&e)))?;
    let config =
        store::remove_persisted(id).map_err(|error| map_mutation_error(error, not_found))?;
    state.daemon.config.config_changed(ConfigKind::Shortcuts);
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

fn map_mutation_error(
    error: store::MutationError,
    rejected: fn(&str) -> Response,
) -> Box<Response> {
    match error {
        store::MutationError::Load(_) => Box::new(load_failed()),
        store::MutationError::Lock => Box::new(save_failed()),
        store::MutationError::Rejected(message) => Box::new(rejected(&message)),
        store::MutationError::Save(_) => Box::new(save_failed()),
    }
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
