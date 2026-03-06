use axum::http::StatusCode;

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

fn repo_url(id: &str) -> String {
    format!("https://github.com/qol-tools/{}.git", id)
}

fn success_uninstall_result(message: &str) -> UninstallResult {
    UninstallResult {
        success: true,
        message: message.to_string(),
    }
}

fn failed_uninstall_result(message: String) -> UninstallResult {
    UninstallResult {
        success: false,
        message,
    }
}
