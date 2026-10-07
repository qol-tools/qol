use crate::features::plugin_store::installer::PluginInstaller;

use super::super::super::helpers::reload_plugin_and_notify;
use super::super::super::types::AppState;

pub(super) async fn uninstall_plugin(state: &AppState, id: &str) -> Result<(), String> {
    log::info!("Uninstall requested for plugin: {}", id);
    let unlinked_dev = unlink_dev_plugin_if_linked(id).map_err(|error| {
        log::error!("Failed to unlink dev-linked plugin {}: {}", id, error);
        format!("Failed to unlink dev-linked plugin: {}", error)
    })?;
    let installer = PluginInstaller::new(state.plugins_dir.clone());
    match installer.uninstall(id).await {
        Ok(()) => {}
        Err(error) if can_ignore_uninstall_error(&error.to_string(), unlinked_dev) => {}
        Err(error) => {
            log::error!("Failed to uninstall plugin {}: {}", id, error);
            return Err("Uninstall failed".to_string());
        }
    }
    reload_plugin_and_notify(state, id);
    tokio::task::spawn_blocking(crate::settings_surface::plugins_changed);
    log::info!("Plugin {} uninstalled successfully", id);
    Ok(())
}

fn can_ignore_uninstall_error(error: &str, unlinked_dev: bool) -> bool {
    unlinked_dev && error.contains("Plugin not installed")
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
