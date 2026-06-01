use axum::http::StatusCode;

use crate::features::plugin_store::installer::PluginInstaller;

use super::super::super::helpers::{read_plugin_version, reload_manager_and_notify};
use super::super::super::types::{AppState, PluginInfo};
use super::repo_url;

pub(super) async fn install_plugin(
    state: &AppState,
    id: &str,
) -> Result<PluginInfo, (StatusCode, String)> {
    log::info!("Install requested for plugin: {}", id);
    ensure_plugins_dir(state)?;
    let installer = PluginInstaller::new(state.plugins_dir.clone());
    installer
        .install(&repo_url(id), id)
        .await
        .map_err(|error| {
            log::error!("Failed to install plugin {}: {}", id, error);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Installation failed: {:#}", error),
            )
        })?;
    reload_manager_and_notify(state);
    log::info!("Plugin {} installed successfully", id);
    Ok(installed_plugin_info(state, id))
}

fn ensure_plugins_dir(state: &AppState) -> Result<(), (StatusCode, String)> {
    std::fs::create_dir_all(&state.plugins_dir).map_err(|error| {
        log::error!("Failed to get plugins directory: {}", error);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to access plugins directory".to_string(),
        )
    })
}

fn installed_plugin_info(state: &AppState, id: &str) -> PluginInfo {
    let version =
        read_plugin_version(&state.plugins_dir.join(id)).unwrap_or_else(|_| "unknown".into());
    PluginInfo {
        id: id.to_string(),
        name: id.to_string(),
        description: "Installed successfully".to_string(),
        installed_version: Some(version.clone()),
        version,
        installed: true,
    }
}
