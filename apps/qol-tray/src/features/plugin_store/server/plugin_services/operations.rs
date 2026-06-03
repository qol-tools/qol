use axum::http::StatusCode;

use super::super::super::source::{resolve_source_for_plugin, PluginSource};
use super::super::types::{AppState, PluginInfo, UninstallResult};

mod install;
mod uninstall;
mod update;

pub(super) async fn install_plugin(
    state: &AppState,
    id: &str,
) -> Result<PluginInfo, (StatusCode, String)> {
    install::install_plugin(state, id).await
}

pub(super) async fn update_plugin(state: &AppState, id: &str) -> UninstallResult {
    update::update_plugin(state, id).await
}

pub(super) async fn uninstall_plugin(state: &AppState, id: &str) -> UninstallResult {
    uninstall::uninstall_plugin(state, id).await
}

pub(super) fn source_for(id: &str) -> Result<PluginSource, (StatusCode, String)> {
    resolve_source_for_plugin(id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("No plugin source provides {}", id),
        )
    })
}

pub(super) fn success_uninstall_result(message: &str) -> UninstallResult {
    UninstallResult {
        success: true,
        message: message.to_string(),
    }
}

pub(super) fn failed_uninstall_result(message: String) -> UninstallResult {
    UninstallResult {
        success: false,
        message,
    }
}
