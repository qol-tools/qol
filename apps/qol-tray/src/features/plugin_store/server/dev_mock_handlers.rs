#![cfg(feature = "dev")]

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

use super::dev_services;
use super::types::{AppState, MockTargetInfo};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/dev/mock-check-update", get(mock_check_update))
        .route("/dev/mock-targets", get(list_mock_targets))
        .route("/dev/mock-targets/start", post(start_mock_targets))
        .route("/dev/mock-targets/stop", post(stop_mock_targets))
        .route("/dev/mock-plugin-build", post(mock_plugin_build))
        .route("/dev/mock-plugin-build/stop", post(stop_mock_plugin_build))
        .route("/dev/mock-self-recompile", post(mock_self_recompile))
        .route("/dev/mock-self-recompile/stop", post(stop_mock_self_recompile))
        .route("/dev/mock-self-update", post(mock_self_update))
        .route("/dev/mock-self-update/stop", post(stop_mock_self_update))
}

pub(super) async fn mock_check_update() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "available": true, "latest": "99.0.0" }))
}

pub(super) async fn list_mock_targets(State(state): State<AppState>) -> Json<Vec<MockTargetInfo>> {
    Json(state.runtime.list_mock_targets())
}

pub(super) async fn start_mock_targets(State(state): State<AppState>) -> impl IntoResponse {
    let started = match dev_services::start_mock_targets(&state) {
        Ok(started) => started,
        Err(message) => return (StatusCode::CONFLICT, message).into_response(),
    };
    mock_targets_response(
        StatusCode::ACCEPTED,
        "started",
        started,
        state.runtime.list_mock_targets(),
    )
}

pub(super) async fn stop_mock_targets(State(state): State<AppState>) -> impl IntoResponse {
    let stopped = dev_services::stop_mock_targets(&state);
    if stopped.is_empty() {
        return mock_targets_response(
            StatusCode::OK,
            "stopped",
            stopped,
            state.runtime.list_mock_targets(),
        );
    }
    mock_targets_response(
        StatusCode::ACCEPTED,
        "stopped",
        stopped,
        state.runtime.list_mock_targets(),
    )
}

pub(super) async fn mock_self_update(State(state): State<AppState>) -> impl IntoResponse {
    mock_start_response(
        state
            .runtime
            .start_mock_self_update(state.daemon.events.clone()),
        "Mock update queued",
    )
}

pub(super) async fn stop_mock_self_update(State(state): State<AppState>) -> impl IntoResponse {
    mock_stop_response(
        state.runtime.stop_mock_self_update(),
        "Stopping mock update",
        "No mock update in progress",
    )
}

pub(super) async fn mock_self_recompile(State(state): State<AppState>) -> impl IntoResponse {
    mock_start_response(
        state
            .runtime
            .start_mock_self_recompile(state.daemon.events.clone()),
        "Mock recompile queued",
    )
}

pub(super) async fn stop_mock_self_recompile(State(state): State<AppState>) -> impl IntoResponse {
    mock_stop_response(
        state.runtime.stop_mock_self_recompile(),
        "Stopping mock recompile",
        "No mock recompile in progress",
    )
}

pub(super) async fn stop_mock_plugin_build(State(state): State<AppState>) -> impl IntoResponse {
    mock_stop_response(
        state.runtime.stop_mock_plugin_build(),
        "Stopping mock build",
        "No mock build in progress",
    )
}

pub(super) async fn mock_plugin_build(State(state): State<AppState>) -> impl IntoResponse {
    mock_start_response(
        dev_services::queue_mock_plugin_build(&state),
        "Mock build queued",
    )
}

fn mock_start_response(
    result: Result<(), &'static str>,
    queued_message: &'static str,
) -> axum::response::Response {
    match result {
        Ok(()) => (StatusCode::ACCEPTED, queued_message).into_response(),
        Err(message) => (StatusCode::CONFLICT, message).into_response(),
    }
}

fn mock_stop_response(
    stopped: bool,
    stopping_message: &'static str,
    idle_message: &'static str,
) -> axum::response::Response {
    if stopped {
        return (StatusCode::ACCEPTED, stopping_message).into_response();
    }
    (StatusCode::OK, idle_message).into_response()
}

fn mock_targets_response(
    status: StatusCode,
    key: &'static str,
    ids: Vec<&'static str>,
    targets: Vec<MockTargetInfo>,
) -> axum::response::Response {
    (
        status,
        Json(serde_json::json!({ key: ids, "targets": targets })),
    )
        .into_response()
}
