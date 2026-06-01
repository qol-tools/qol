use crate::features::plugin_store::installer::PluginInstaller;

use super::super::super::helpers::reload_manager_and_notify;
use super::super::super::types::{AppState, UninstallResult};
use super::{failed_uninstall_result, success_uninstall_result};

pub(super) async fn uninstall_plugin(state: &AppState, id: &str) -> UninstallResult {
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

async fn uninstall_installed_copy(
    installer: &PluginInstaller,
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

fn uninstall_message(removed_installed_copy: bool, unlinked_dev: bool) -> String {
    match (removed_installed_copy, unlinked_dev) {
        (true, true) => "Uninstalled and unlinked successfully".to_string(),
        (true, false) => "Uninstalled successfully".to_string(),
        (false, true) => "Unlinked successfully".to_string(),
        (false, false) => "Uninstall completed".to_string(),
    }
}

#[cfg(feature = "dev")]
fn unlink_dev_plugin_if_linked(plugin_id: &str) -> Result<bool, String> {
    let config_dir = super::super::super::helpers::shared_config_dir()?;
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
