use axum::http::StatusCode;

use super::types::{
    AppState, InstalledPluginsResponse, PluginInfo, PluginsResponse, UninstallResult,
};

mod catalog;
mod installed;
mod operations;

pub(super) async fn list_plugins(refresh: bool) -> Result<PluginsResponse, (StatusCode, String)> {
    catalog::list_plugins(refresh).await
}

pub(super) async fn install_plugin(
    state: &AppState,
    id: &str,
) -> Result<PluginInfo, (StatusCode, String)> {
    operations::install_plugin(state, id).await
}

pub(super) async fn update_plugin(state: &AppState, id: &str) -> UninstallResult {
    operations::update_plugin(state, id).await
}

pub(super) async fn uninstall_plugin(state: &AppState, id: &str) -> UninstallResult {
    operations::uninstall_plugin(state, id).await
}

pub(super) fn list_installed(state: &AppState) -> Result<InstalledPluginsResponse, StatusCode> {
    installed::list_installed(state)
}
