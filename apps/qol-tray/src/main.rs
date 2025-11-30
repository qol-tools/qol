mod features;

use anyhow::Result;
use features::plugin_manager::PluginManager;
use features::tray::TrayManager;
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    log::info!("Starting QoL Tray daemon...");

    let mut plugin_manager = PluginManager::new();
    plugin_manager.load_plugins()?;

    let plugin_manager = Arc::new(Mutex::new(plugin_manager));

    let _tray = TrayManager::new(Arc::clone(&plugin_manager))?;

    log::info!("QoL Tray daemon started successfully");

    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }
}
