use crate::plugins::PluginManager;
use crate::tray::TrayManager;
use anyhow::Result;
use std::sync::{Arc, Mutex};

pub(super) fn run_app<F>(init: F) -> Result<()>
where
    F: FnOnce() -> Result<(TrayManager, Arc<Mutex<PluginManager>>)>,
{
    let (mut tray, plugin_manager) = init()?;
    let _signal_listener = crate::signal::install_signal_handler(tray.shutdown_sender())?;

    tray.wait_for_shutdown();

    super::shutdown_plugins(&plugin_manager);
    drop(plugin_manager);
    log::info!("Shutdown signal received, exiting...");
    Ok(())
}
