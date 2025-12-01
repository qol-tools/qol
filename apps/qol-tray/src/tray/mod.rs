mod platform;
mod icon;

use crate::plugins::PluginManager;
use crate::features::FeatureRegistry;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

pub struct TrayManager {
    _tray: platform::PlatformTray,
}

impl TrayManager {
    pub fn new(
        plugin_manager: Arc<Mutex<PluginManager>>,
        feature_registry: Arc<FeatureRegistry>,
        shutdown_tx: broadcast::Sender<()>,
    ) -> Result<Self> {
        let icon = icon::create_icon();
        let tray = platform::create_tray(plugin_manager, feature_registry, shutdown_tx, icon)?;
        Ok(Self { _tray: tray })
    }
}
