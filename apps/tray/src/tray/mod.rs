pub mod icon;
pub mod platform;
pub mod tone;

use crate::daemon::EventBus;
use crate::features::FeatureRegistry;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct TrayManager {
    _tray: platform::PlatformTray,
    shutdown_tx: broadcast::Sender<()>,
    _post_pull_task: Option<tokio::task::JoinHandle<()>>,
}

impl TrayManager {
    pub fn new(
        feature_registry: Arc<FeatureRegistry>,
        shutdown_tx: broadcast::Sender<()>,
        shutdown_rx: broadcast::Receiver<()>,
        update_available: bool,
        events: Arc<EventBus>,
        post_pull_task: Option<tokio::task::JoinHandle<()>>,
    ) -> Result<Self> {
        let icon = icon::tray_icon(update_available, tone::current());
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
            _post_pull_task: post_pull_task,
        })
    }

    pub(crate) fn shutdown_sender(&self) -> broadcast::Sender<()> {
        self.shutdown_tx.clone()
    }
}
