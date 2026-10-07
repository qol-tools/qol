use crate::features::plugin_store::installer::PluginInstaller;

use super::super::super::helpers::reload_plugin_and_notify;
use super::super::super::types::AppState;
use super::source_for;

pub(super) async fn install_plugin(state: &AppState, id: &str) -> Result<(), String> {
    log::info!("Install requested for plugin: {}", id);
    std::fs::create_dir_all(&state.plugins_dir).map_err(|error| {
        log::error!("Failed to get plugins directory: {}", error);
        "Failed to access plugins directory".to_string()
    })?;
    let source = source_for(id)?;
    let installer = PluginInstaller::new(state.plugins_dir.clone());
    installer.install(&source, id).await.map_err(|error| {
        log::error!("Failed to install plugin {}: {}", id, error);
        crate::updates::plain_update_failure(&error)
    })?;
    reload_plugin_and_notify(state, id);
    tokio::task::spawn_blocking(crate::settings_surface::plugins_changed);
    log::info!("Plugin {} installed successfully", id);
    Ok(())
}
