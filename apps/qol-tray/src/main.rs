mod features;
mod hotkeys;
mod menu;
mod paths;
mod plugins;
mod tray;
mod updates;
mod version;

use anyhow::Result;
use plugins::{PluginManager, PluginLoader};
use tray::TrayManager;
use features::FeatureRegistry;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    log::info!("Starting QoL Tray daemon...");

    let update_available = match tokio::time::timeout(
        Duration::from_secs(2),
        updates::check_for_updates()
    ).await {
        Ok(Ok(has_update)) => has_update,
        Ok(Err(e)) => {
            log::debug!("Update check failed: {}", e);
            false
        }
        Err(_) => {
            log::debug!("Update check timed out");
            false
        }
    };

    let (shutdown_tx, mut shutdown_rx) = broadcast::channel::<()>(1);

    let mut plugin_manager = PluginManager::new();
    plugin_manager.load_plugins()?;
    let plugin_manager = Arc::new(Mutex::new(plugin_manager));

    let mut feature_registry = FeatureRegistry::new();
    feature_registry.register(Box::new(features::plugin_store::PluginStore::new()));
    let feature_registry = Arc::new(feature_registry);

    features::plugin_store::PluginStore::start_server(plugin_manager.clone()).await?;

    if let Ok(plugins_dir) = PluginLoader::default_plugin_dir() {
        if let Err(e) = hotkeys::start_hotkey_listener(plugins_dir) {
            log::warn!("Failed to start hotkey listener: {}", e);
        }
    }

    let _tray = TrayManager::new(feature_registry, shutdown_tx, update_available)?;

    log::info!("QoL Tray daemon started successfully");

    shutdown_rx.recv().await.ok();
    drop(plugin_manager);
    log::info!("Shutdown signal received, exiting...");
    Ok(())
}
