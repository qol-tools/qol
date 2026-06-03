use crate::features::plugin_store::installer::PluginInstaller;

use super::super::super::helpers::{read_plugin_version, reload_manager_and_notify};
use super::super::super::types::{AppState, UninstallResult};
use super::{failed_uninstall_result, source_for, success_uninstall_result};

pub(super) async fn update_plugin(state: &AppState, id: &str) -> UninstallResult {
    log::info!("Update requested for plugin: {}", id);
    let source = match source_for(id) {
        Ok(source) => source,
        Err((_, message)) => return failed_uninstall_result(message),
    };
    let installer = PluginInstaller::new(state.plugins_dir.clone());
    if let Err(error) = installer.update(&source, id).await {
        log::error!("Failed to update plugin {}: {}", id, error);
        return failed_uninstall_result(format!("Update failed: {:#}", error));
    }
    update_cached_version(state, id);
    reload_manager_and_notify(state);
    log::info!("Plugin {} updated successfully", id);
    success_uninstall_result("Updated successfully")
}

fn update_cached_version(state: &AppState, id: &str) {
    if let Ok(version) = read_plugin_version(&state.plugins_dir.join(id)) {
        crate::features::plugin_store::github::update_cached_version(id, &version);
        if let Ok(mut guard) = state.plugins_cache.write() {
            if let Some(cache) = guard.as_mut() {
                if let Some(plugin) = cache.plugins.iter_mut().find(|p| p.id == id) {
                    plugin.version = version;
                }
            }
        }
    }
}
