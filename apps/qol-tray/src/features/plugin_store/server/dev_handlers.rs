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
        .and_then(|path| resolve_selected_worktree(&path).map(|w| w.branch));

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
    let requested_path = body.and_then(|Json(req)| req.worktree_path);
    let worktree_path = requested_path
        .as_deref()
        .and_then(|path| resolve_selected_worktree(path).map(|w| PathBuf::from(w.path)));

    let requested_unknown = requested_path
        .as_deref()
        .is_some_and(|path| resolve_selected_worktree(path).is_none());
    if requested_unknown {
        return (StatusCode::BAD_REQUEST, "Unknown worktree path").into_response();
    }

    dev_services::queue_self_recompile(&state, worktree_path)
        .map(|_| (StatusCode::ACCEPTED, ""))
        .unwrap_or_else(|msg| (StatusCode::CONFLICT, msg))
        .into_response()
}

fn resolve_selected_worktree(path: &str) -> Option<super::types::WorktreeInfo> {
    let selected = std::path::Path::new(path);
    resolve_selected_worktree_from(dev_services::list_worktrees(), selected)
}

fn resolve_selected_worktree_from(
    worktrees: Vec<super::types::WorktreeInfo>,
    selected: &std::path::Path,
) -> Option<super::types::WorktreeInfo> {
    worktrees
        .into_iter()
        .find(|worktree| worktree_matches_selection(worktree, selected))
}

fn worktree_matches_selection(
    worktree: &super::types::WorktreeInfo,
    selected: &std::path::Path,
) -> bool {
    let worktree_path = std::path::Path::new(&worktree.path);
    if worktree_path == selected {
        return true;
    }
    worktree_path.parent() == Some(selected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn worktree(path: std::path::PathBuf) -> super::super::types::WorktreeInfo {
        super::super::types::WorktreeInfo {
            branch: "feat/config-contract-v1".to_string(),
            path: path.to_string_lossy().into_owned(),
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn prop_worktree_matches_exact_and_parent_selection(
            root in "[a-z0-9_-]{1,12}",
            feature in "[a-z0-9_-]{1,12}",
            repo in "[a-z0-9_-]{1,12}",
        ) {
            let worktree_path = std::path::PathBuf::from("/tmp")
                .join(root)
                .join(feature)
                .join(repo);
            let worktree = worktree(worktree_path.clone());

            prop_assert!(worktree_matches_selection(&worktree, &worktree_path));

            let parent = worktree_path.parent().unwrap().to_path_buf();
            prop_assert!(worktree_matches_selection(&worktree, &parent));
        }

        #[test]
        fn prop_worktree_rejects_unrelated_selection(
            root in "[a-z0-9_-]{1,12}",
            feature in "[a-z0-9_-]{1,12}",
            repo in "[a-z0-9_-]{1,12}",
            other_feature in "[a-z0-9_-]{1,12}",
            other_repo in "[a-z0-9_-]{1,12}",
        ) {
            let worktree_path = std::path::PathBuf::from("/tmp")
                .join(root.clone())
                .join(feature.clone())
                .join(repo.clone());
            let unrelated = std::path::PathBuf::from("/tmp")
                .join(root)
                .join(other_feature)
                .join(other_repo);

            prop_assume!(unrelated != worktree_path);
            prop_assume!(Some(unrelated.as_path()) != worktree_path.parent());

            let worktree = worktree(worktree_path);
            prop_assert!(!worktree_matches_selection(&worktree, &unrelated));
        }
    }

    #[test]
    fn resolve_selected_worktree_from_accepts_feature_dir_selection() {
        let feature_dir = std::path::PathBuf::from("/tmp/feat-config-contract-v1");
        let repo_dir = feature_dir.join("qol-tray");
        let selected =
            resolve_selected_worktree_from(vec![worktree(repo_dir.clone())], &feature_dir).unwrap();

        assert_eq!(selected.path, repo_dir.to_string_lossy());
    }
}
