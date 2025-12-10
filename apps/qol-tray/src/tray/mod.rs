pub mod platform;
pub mod icon;

use crate::features::FeatureRegistry;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct TrayManager {
    _tray: platform::PlatformTray,
}

impl TrayManager {
    pub fn new(
        feature_registry: Arc<FeatureRegistry>,
        shutdown_tx: broadcast::Sender<()>,
        shutdown_rx: broadcast::Receiver<()>,
        update_available: bool,
    ) -> Result<Self> {
        let icon = if update_available {
            icon::create_icon_with_dot()
        } else {
            icon::create_icon()
        };
        let tray = platform::create_tray(
            feature_registry,
            shutdown_tx,
            shutdown_rx,
            icon,
            update_available,
        )?;
        Ok(Self { _tray: tray })
    }
}
