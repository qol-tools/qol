use axum::{
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use super::super::helpers::reload_manager_and_notify_without_profile_sync;
use super::super::types::{AppState, MAX_CONFIG_SIZE};
use super::http_json::parse_json_body;

pub(in super::super) async fn get_sync_status(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.sync_service.status())
}

pub(in super::super) async fn connect_sync(
    State(state): State<AppState>,
    body: Bytes,
) -> impl IntoResponse {
    let request = match parse_json_body::<crate::sync::SyncConnectRequest>(body, MAX_CONFIG_SIZE) {
        Ok(request) => request,
        Err(response) => return *response,
    };
    match state.sync_service.connect(request).await {
        Ok(result) => sync_result_response(&state, result),
        Err(error) => sync_error_response(error),
    }
}

pub(in super::super) async fn pull_sync(State(state): State<AppState>) -> impl IntoResponse {
    match state.sync_service.manual_pull().await {
        Ok(result) => sync_result_response(&state, result),
        Err(error) => sync_error_response(error),
    }
}

pub(in super::super) async fn push_sync(State(state): State<AppState>) -> impl IntoResponse {
    match state.sync_service.manual_push().await {
        Ok(result) => sync_result_response(&state, result),
        Err(error) => sync_error_response(error),
    }
}

pub(in super::super) async fn disconnect_sync(State(state): State<AppState>) -> impl IntoResponse {
    match state.sync_service.disconnect().await {
        Ok(result) => Json(result).into_response(),
        Err(error) => sync_error_response(error),
    }
}

pub(in super::super) async fn acknowledge_sync(State(state): State<AppState>) -> impl IntoResponse {
    match state.sync_service.acknowledge_incident().await {
        Ok(result) => Json(result).into_response(),
        Err(error) => sync_error_response(error),
    }
}

pub(in super::super) async fn open_sync_backups_dir(
    State(state): State<AppState>,
) -> impl IntoResponse {
    match state.sync_service.open_backups_dir() {
        Ok(()) => StatusCode::OK.into_response(),
        Err(error) => sync_error_response(error),
    }
}

fn sync_result_response(state: &AppState, result: crate::sync::SyncActionResult) -> Response {
    if result.applied_remote {
        reload_manager_and_notify_without_profile_sync(state);
    }
    Json(result).into_response()
}

fn sync_error_response(error: anyhow::Error) -> Response {
    let message = error.to_string();
    let status = if looks_like_bad_request(&message) {
        StatusCode::BAD_REQUEST
    } else if looks_like_upstream_error(&message) {
        StatusCode::BAD_GATEWAY
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status, message).into_response()
}

fn looks_like_bad_request(message: &str) -> bool {
    let normalized = message.to_lowercase();
    normalized.contains("required")
        || normalized.contains("invalid")
        || normalized.contains("not configured")
        || normalized.contains("cannot be empty")
        || normalized.contains("unsupported")
}

fn looks_like_upstream_error(message: &str) -> bool {
    let normalized = message.to_lowercase();
    normalized.contains("github")
        || normalized.contains("upstream")
        || normalized.contains("authentication failed")
}
