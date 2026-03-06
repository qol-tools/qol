use axum::http::StatusCode;

use super::super::helpers::{read_plugin_version, reload_manager_and_notify};
use super::super::types::{AppState, PluginInfo, UninstallResult};

pub(super) async fn install_plugin(
    state: &AppState,
    id: &str,
) -> Result<PluginInfo, (StatusCode, String)> {
    use super::super::super::installer::PluginInstaller;

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

pub(super) async fn update_plugin(state: &AppState, id: &str) -> UninstallResult {
    use super::super::super::installer::PluginInstaller;

    log::info!("Update requested for plugin: {}", id);
    let installer = PluginInstaller::new(state.plugins_dir.clone());
    if let Err(error) = installer.update(&repo_url(id), id).await {
        log::error!("Failed to update plugin {}: {}", id, error);
        return failed_uninstall_result(format!("Update failed: {:#}", error));
    }
    update_cached_version(state, id);
    reload_manager_and_notify(state);
    log::info!("Plugin {} updated successfully", id);
    success_uninstall_result("Updated successfully")
}

pub(super) async fn uninstall_plugin(state: &AppState, id: &str) -> UninstallResult {
    use super::super::super::installer::PluginInstaller;

    log::info!("Uninstall requested for plugin: {}", id);
    let unlinked_dev = match unlink_dev_plugin_if_linked(id) {
        Ok(value) => value,
        Err(error) => {
            log::error!("Failed to unlink dev-linked plugin {}: {}", id, error);
            return failed_uninstall_result(format!(
                "Failed to unlink dev-linked plugin: {}",
                error
            ));
        }
    };

    let installer = PluginInstaller::new(state.plugins_dir.clone());
    let removed_installed_copy = match uninstall_installed_copy(&installer, id, unlinked_dev).await
    {
        Ok(value) => value,
        Err(result) => return result,
    };

    reload_manager_and_notify(state);
    log::info!("Plugin {} uninstalled successfully", id);
    success_uninstall_result(&uninstall_message(removed_installed_copy, unlinked_dev))
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

fn update_cached_version(state: &AppState, id: &str) {
    if let Ok(version) = read_plugin_version(&state.plugins_dir.join(id)) {
        super::super::super::github::update_cached_version(id, &version);
    }
}

async fn uninstall_installed_copy(
    installer: &super::super::super::installer::PluginInstaller,
    id: &str,
    unlinked_dev: bool,
) -> Result<bool, UninstallResult> {
    match installer.uninstall(id).await {
        Ok(()) => Ok(true),
        Err(error) if can_ignore_uninstall_error(&error.to_string(), unlinked_dev) => Ok(false),
        Err(error) => {
            log::error!("Failed to uninstall plugin {}: {}", id, error);
            Err(failed_uninstall_result("Uninstall failed".to_string()))
        }
    }
}

fn can_ignore_uninstall_error(error: &str, unlinked_dev: bool) -> bool {
    unlinked_dev && error.contains("Plugin not installed")
}

fn repo_url(id: &str) -> String {
    format!("https://github.com/qol-tools/{}.git", id)
}

fn uninstall_message(removed_installed_copy: bool, unlinked_dev: bool) -> String {
    match (removed_installed_copy, unlinked_dev) {
        (true, true) => "Uninstalled and unlinked successfully".to_string(),
        (true, false) => "Uninstalled successfully".to_string(),
        (false, true) => "Unlinked successfully".to_string(),
        (false, false) => "Uninstall completed".to_string(),
    }
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

#[cfg(feature = "dev")]
fn unlink_dev_plugin_if_linked(plugin_id: &str) -> Result<bool, String> {
    let config_dir = super::super::helpers::shared_config_dir()?;
    match crate::dev::remove_link(plugin_id, &config_dir) {
        Ok(()) => Ok(true),
        Err(error) if error.contains("not dev-linked") => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(not(feature = "dev"))]
fn unlink_dev_plugin_if_linked(_plugin_id: &str) -> Result<bool, String> {
    Ok(false)
}
