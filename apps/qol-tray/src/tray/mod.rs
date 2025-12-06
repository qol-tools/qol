mod platform;
pub mod icon;

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
        update_available: bool,
    ) -> Result<Self> {
        let icon = if update_available {
            icon::create_icon_with_dot()
        } else {
            icon::create_icon()
        };
        let tray = platform::create_tray(plugin_manager, feature_registry, shutdown_tx, icon, update_available)?;
        Ok(Self { _tray: tray })
    }
}
