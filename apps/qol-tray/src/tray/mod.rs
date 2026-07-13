pub mod icon;
pub mod platform;

use crate::daemon::EventBus;
use crate::features::FeatureRegistry;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct TrayManager {
    _tray: platform::PlatformTray,
    shutdown_tx: broadcast::Sender<()>,
}

impl TrayManager {
    pub fn new(
        feature_registry: Arc<FeatureRegistry>,
        shutdown_tx: broadcast::Sender<()>,
        shutdown_rx: broadcast::Receiver<()>,
        update_available: bool,
        events: Arc<EventBus>,
    ) -> Result<Self> {
        let icon = if update_available {
            icon::create_icon_with_dot()
        } else {
            icon::create_icon()
        };
        let tray = platform::create_tray(
            feature_registry,
            shutdown_tx.clone(),
            shutdown_rx,
            icon,
            update_available,
            events,
        )?;
        Ok(Self {
            _tray: tray,
            shutdown_tx,
        })
    }

    pub(crate) fn shutdown_sender(&self) -> broadcast::Sender<()> {
        self.shutdown_tx.clone()
    }
}
