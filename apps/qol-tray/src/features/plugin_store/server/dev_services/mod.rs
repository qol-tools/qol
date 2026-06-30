mod mock;
mod recompile;
mod reload;
mod worktrees;

use super::types::AppState;

pub(super) fn queue_reload(
    state: &AppState,
    worktree_branch: Option<String>,
) -> Result<(), &'static str> {
    reload::queue_reload(state, worktree_branch)
}

pub(super) fn queue_reload_single(state: &AppState, plugin_id: String) -> Result<(), &'static str> {
    reload::queue_reload_single(state, plugin_id)
}

pub(super) fn refresh_discovery(state: &AppState) {
    crate::dev::state::start_discovery(
        &state.dev_state,
        &state.daemon.events,
        state.plugins_dir.clone(),
    );
}

pub(super) fn queue_self_recompile(
    state: &AppState,
    worktree_path: Option<std::path::PathBuf>,
) -> Result<(), &'static str> {
    recompile::queue_self_recompile(state, worktree_path)
}

pub(super) fn restart_prebuilt(state: &AppState) -> Result<(), &'static str> {
    if !state.runtime.try_mark_restart_pending() {
        return Err("Restart already pending");
    }
    let Some(binary) = state.restart.resolve_restart_binary() else {
        state.runtime.clear_restart_pending();
        return Err("Restart binary not found");
    };
    let restart = state.restart.clone();
    let runtime = state.runtime.clone();
    std::env::set_var(qol_conventions::ENV_DEV_ROLLING_RESTART, "1");
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        if let Err(error) = restart.exec_restart(&binary) {
            log::error!(
                "Rolling dev restart exec failed for {}: {}",
                binary.display(),
                error
            );
            runtime.clear_restart_pending();
            std::process::exit(1);
        }
    });
    Ok(())
}

pub(super) async fn promote_generation(state: &AppState) -> Result<(), String> {
    super::promote_shadow_to_stable(state.clone())
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub(super) fn start_mock_targets(state: &AppState) -> Result<Vec<&'static str>, &'static str> {
    mock::start_mock_targets(state)
}

pub(super) fn queue_mock_plugin_build(state: &AppState) -> Result<(), &'static str> {
    mock::queue_mock_plugin_build(state)
}

pub(super) fn stop_mock_targets(state: &AppState) -> Vec<&'static str> {
    mock::stop_mock_targets(state)
}

pub(super) fn list_worktrees() -> Vec<super::types::WorktreeInfo> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    worktrees::scan(&root)
}

pub(super) fn list_branches() -> Vec<String> {
    list_worktrees().into_iter().map(|w| w.branch).collect()
}

pub(super) fn current_repo_branch() -> Option<String> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    worktrees::resolve_git_branch(&root)
}
