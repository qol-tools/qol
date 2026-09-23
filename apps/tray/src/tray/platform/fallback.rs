use crate::daemon::EventBus;
use crate::features::FeatureRegistry;
use crate::plugins::PluginManager;
use crate::tray::TrayManager;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tray_icon::Icon;

pub enum PlatformTray {}

pub(crate) fn request_shutdown(shutdown_tx: &broadcast::Sender<()>) {
    let _ = shutdown_tx.send(());
}

pub fn create_tray(
    _feature_registry: Arc<FeatureRegistry>,
    _shutdown_tx: broadcast::Sender<()>,
    _shutdown_rx: broadcast::Receiver<()>,
    _icon: Icon,
    _update_available: bool,
    _events: Arc<EventBus>,
) -> Result<PlatformTray> {
    Err(unsupported_error())
}

pub fn run_app<F>(_init: F) -> Result<()>
where
    F: FnOnce() -> Result<(TrayManager, Arc<Mutex<PluginManager>>)>,
{
    Err(unsupported_error())
}

fn unsupported_error() -> anyhow::Error {
    std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "qol-tray is unsupported on this platform",
    )
    .into()
}
