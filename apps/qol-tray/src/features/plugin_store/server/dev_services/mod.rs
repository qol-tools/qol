mod mock;
mod recompile;
mod reload;

use super::types::AppState;

pub(super) fn queue_reload(state: &AppState) -> Result<(), &'static str> {
    reload::queue_reload(state)
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

pub(super) fn queue_self_recompile(state: &AppState) -> Result<(), &'static str> {
    recompile::queue_self_recompile(state)
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
