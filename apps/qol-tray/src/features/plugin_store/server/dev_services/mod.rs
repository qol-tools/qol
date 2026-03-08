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
