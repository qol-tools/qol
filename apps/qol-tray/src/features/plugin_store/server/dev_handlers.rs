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

pub(super) async fn reload_plugins(
    State(state): State<AppState>,
    body: Option<Json<super::types::ReloadRequest>>,
) -> impl IntoResponse {
    let worktree_branch = body
        .and_then(|Json(req)| req.worktree_path)
        .and_then(|path| {
            dev_services::list_worktrees()
                .into_iter()
                .find(|w| w.path == path)
                .map(|w| w.branch)
        });

    dev_services::queue_reload(&state, worktree_branch)
        .map(|_| (StatusCode::OK, "Reload queued"))
        .unwrap_or_else(|msg| {
            log::warn!("Developer reload requested, but a build is already in progress");
            (StatusCode::CONFLICT, msg)
        })
        .into_response()
}

async fn reload_single_plugin(
    Path(plugin_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    dev_services::queue_reload_single(&state, plugin_id)
        .map(|_| (StatusCode::OK, "Reload queued"))
        .unwrap_or_else(|msg| {
            log::warn!("Developer reload requested, but a build is already in progress");
            (StatusCode::CONFLICT, msg)
        })
        .into_response()
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

    let is_valid_worktree = worktree_path.as_ref().is_none_or(|path| {
        dev_services::list_worktrees()
            .iter()
            .any(|w| std::path::Path::new(&w.path) == path.as_path())
    });

    if !is_valid_worktree {
        return (StatusCode::BAD_REQUEST, "Unknown worktree path").into_response();
    }

    dev_services::queue_self_recompile(&state, worktree_path)
        .map(|_| (StatusCode::ACCEPTED, ""))
        .unwrap_or_else(|msg| (StatusCode::CONFLICT, msg))
        .into_response()
}
