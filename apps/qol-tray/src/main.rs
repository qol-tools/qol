mod features;
mod hotkeys;
mod menu;
mod paths;
mod plugins;
mod tray;
mod updates;
mod version;

use anyhow::Result;
use features::FeatureRegistry;
use plugins::{PluginLoader, PluginManager};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;
use tray::TrayManager;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    log::info!("Starting QoL Tray daemon...");
    tray::platform::run_app(app_init)
}

fn app_init() -> Result<(TrayManager, Arc<Mutex<PluginManager>>)> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let (shutdown_tx, shutdown_rx, update_available, plugin_manager, feature_registry) =
        rt.block_on(async_init())?;

    // Keep tokio runtime alive in background
    std::thread::spawn(move || {
        rt.block_on(std::future::pending::<()>());
    });

    let tray = TrayManager::new(feature_registry, shutdown_tx, shutdown_rx, update_available)?;

    log::info!("QoL Tray daemon started successfully");
    Ok((tray, plugin_manager))
}

async fn async_init() -> Result<(
    broadcast::Sender<()>,
    broadcast::Receiver<()>,
    bool,
    Arc<Mutex<PluginManager>>,
    Arc<FeatureRegistry>,
)> {
    let update_available = check_for_updates().await;

    let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);

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

    Ok((
        shutdown_tx,
        shutdown_rx,
        update_available,
        plugin_manager,
        feature_registry,
    ))
}

async fn check_for_updates() -> bool {
    match tokio::time::timeout(Duration::from_secs(2), updates::check_for_updates()).await {
        Ok(Ok(has_update)) => has_update,
        Ok(Err(e)) => {
            log::debug!("Update check failed: {}", e);
            false
        }
        Err(_) => {
            log::debug!("Update check timed out");
            false
        }
    }
}
