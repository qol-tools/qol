use axum::http::StatusCode;

use super::types::{AppState, InstalledPluginsResponse, PluginsResponse};

mod catalog;
mod installed;
mod operations;

pub(super) fn list_plugins(
    state: &AppState,
    refresh: bool,
) -> Result<PluginsResponse, (StatusCode, String)> {
    catalog::list_plugins(state, refresh)
}

pub(super) use operations::run_operation;

pub(super) fn list_installed(state: &AppState) -> Result<InstalledPluginsResponse, StatusCode> {
    installed::list_installed(state)
}
