use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::path::PathBuf;

use super::dev_services;
use super::types::AppState;

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/dev/reload", post(reload_plugins))
        .route("/dev/reload/{plugin_id}", post(reload_single_plugin))
        .route("/dev/recompile-self", post(recompile_self))
        .route("/dev/worktrees", get(list_worktrees_handler))
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

async fn list_worktrees_handler() -> impl IntoResponse {
    Json(dev_services::list_worktrees())
}

pub(super) async fn recompile_self(
    State(state): State<AppState>,
    body: Option<Json<super::types::RecompileSelfRequest>>,
) -> impl IntoResponse {
    let worktree_path = body
        .and_then(|Json(req)| req.worktree_path)
        .map(PathBuf::from);
    if let Some(ref path) = worktree_path {
        let known = dev_services::list_worktrees();
        if !known
            .iter()
            .any(|w| std::path::Path::new(&w.path) == path.as_path())
        {
            return (StatusCode::BAD_REQUEST, "Unknown worktree path").into_response();
        }
    }
    if let Err(message) = dev_services::queue_self_recompile(&state, worktree_path) {
        return (StatusCode::CONFLICT, message).into_response();
    }
    StatusCode::ACCEPTED.into_response()
}
