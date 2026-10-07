use crate::features::plugin_store::installer::PluginInstaller;

use super::super::super::helpers::{read_plugin_version, reload_plugin_and_notify};
use super::super::super::types::AppState;
use super::source_for;

pub(super) async fn update_plugin(state: &AppState, id: &str) -> Result<(), String> {
    log::info!("Update requested for plugin: {}", id);
    let source = source_for(id)?;
    let installer = PluginInstaller::new(state.plugins_dir.clone());
    if let Err(error) = installer.update(&source, id).await {
        log::error!("Failed to update plugin {}: {}", id, error);
        return Err(crate::updates::plain_update_failure(&error));
    }
    update_cached_version(state, id);
    reload_plugin_and_notify(state, id);
    log::info!("Plugin {} updated successfully", id);
    Ok(())
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
