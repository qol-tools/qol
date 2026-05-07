pub mod icon;
pub mod platform;

use crate::daemon::EventBus;
use crate::features::FeatureRegistry;
use crate::shortcuts::watcher::ShortcutWatcher;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::broadcast;

pub struct TrayManager {
    _tray: platform::PlatformTray,
    _shortcuts_watcher: ShortcutWatcher,
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
            shutdown_tx,
            shutdown_rx,
            icon,
            update_available,
            events,
        )?;
        let shortcuts_watcher = ShortcutWatcher::start();
        Ok(Self {
            _tray: tray,
            _shortcuts_watcher: shortcuts_watcher,
        })
    }
}
