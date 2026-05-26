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
        .route("/dev/active-worktree", get(active_worktree_handler))
}

pub(super) async fn reload_plugins(
    State(state): State<AppState>,
    body: Option<Json<super::types::ReloadRequest>>,
) -> impl IntoResponse {
    let worktree_branch = body.and_then(|Json(req)| req.worktree_branch);

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
    Json(dev_services::list_branches())
}

async fn active_worktree_handler() -> impl IntoResponse {
    Json(resolve_active_worktree())
}

fn resolve_active_worktree() -> super::types::ActiveWorktreeResponse {
    let config_dir = super::helpers::shared_config_dir().ok();
    let persisted = config_dir
        .as_ref()
        .and_then(|dir| crate::dev::get_active_worktree_branch(dir));
    let known = dev_services::list_branches();
    let branch = match persisted {
        Some(b) if known.iter().any(|k| k == &b) => Some(b),
        Some(_) => {
            super::helpers::persist_worktree_branch(None);
            None
        }
        None => None,
    };
    let repo_branch = dev_services::current_repo_branch();
    super::types::ActiveWorktreeResponse {
        branch,
        repo_branch,
    }
}

pub(super) async fn recompile_self(
    State(state): State<AppState>,
    body: Option<Json<super::types::RecompileSelfRequest>>,
) -> impl IntoResponse {
    let requested_branch = body.and_then(|Json(req)| req.worktree_branch);
    let worktree_path = requested_branch
        .as_deref()
        .and_then(resolve_path_for_branch);

    if worktree_path.is_some() || requested_branch.is_none() {
        return dev_services::queue_self_recompile(&state, worktree_path)
            .map(|_| (StatusCode::ACCEPTED, ""))
            .unwrap_or_else(|msg| (StatusCode::CONFLICT, msg))
            .into_response();
    }

    dev_services::queue_reload(&state, requested_branch)
        .map(|_| (StatusCode::ACCEPTED, "Plugin reload queued"))
        .unwrap_or_else(|msg| (StatusCode::CONFLICT, msg))
        .into_response()
}

fn resolve_path_for_branch(branch: &str) -> Option<PathBuf> {
    dev_services::list_worktrees()
        .into_iter()
        .find(|w| w.branch == branch)
        .map(|w| PathBuf::from(w.path))
}

#[cfg(test)]
mod tests {
    #[test]
    fn active_worktree_response_serializes_branch_and_repo_branch() {
        let response = super::super::types::ActiveWorktreeResponse {
            branch: Some("feat/x".to_string()),
            repo_branch: Some("main".to_string()),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(json, r#"{"branch":"feat/x","repoBranch":"main"}"#);
    }

    #[test]
    fn active_worktree_response_serializes_null_branch() {
        let response = super::super::types::ActiveWorktreeResponse {
            branch: None,
            repo_branch: Some("main".to_string()),
        };
        let json = serde_json::to_string(&response).unwrap();
        assert_eq!(json, r#"{"branch":null,"repoBranch":"main"}"#);
    }
}
