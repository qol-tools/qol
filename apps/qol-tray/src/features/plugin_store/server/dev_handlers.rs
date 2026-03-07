use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Router,
};

use super::dev_services;
use super::types::AppState;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/dev/reload", post(reload_plugins))
        .route("/dev/reload/{plugin_id}", post(reload_single_plugin))
        .route("/dev/recompile-self", post(recompile_self))
}

pub(super) async fn reload_plugins(State(state): State<AppState>) -> impl IntoResponse {
    if let Err(message) = dev_services::queue_reload(&state) {
        log::warn!("Developer reload requested, but a build is already in progress");
        return (StatusCode::CONFLICT, message).into_response();
    }
    (StatusCode::OK, "Reload queued").into_response()
}

async fn reload_single_plugin(
    Path(plugin_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if let Err(message) = dev_services::queue_reload_single(&state, plugin_id) {
        log::warn!("Developer reload requested, but a build is already in progress");
        return (StatusCode::CONFLICT, message).into_response();
    }
    (StatusCode::OK, "Reload queued").into_response()
}

pub(super) async fn recompile_self(State(state): State<AppState>) -> impl IntoResponse {
    if let Err(message) = dev_services::queue_self_recompile(&state) {
        return (StatusCode::CONFLICT, message).into_response();
    }
    StatusCode::ACCEPTED.into_response()
}
