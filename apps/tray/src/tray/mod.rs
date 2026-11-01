mod headless;
pub mod icon;
pub mod platform;
pub mod tone;

use crate::daemon::EventBus;
use crate::features::FeatureRegistry;
use crate::plugins::PluginManager;
use qol_runtime::protocol::NotificationLevel;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

const MISSING_ICON_TITLE: &str = "QoL Tray is running without its icon";
const MISSING_ICON_NOTICE_WAIT: Duration = Duration::from_secs(30);

pub struct TrayManager {
    _tray: Option<platform::PlatformTray>,
    shutdown_tx: broadcast::Sender<()>,
    headless_shutdown_rx: Option<broadcast::Receiver<()>>,
    _post_pull_task: Option<tokio::task::JoinHandle<()>>,
}

pub fn run_app<F>(init: F) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<(TrayManager, Arc<Mutex<PluginManager>>)>,
{
    if crate::headless_mode::active() {
        headless::run_app(init)
    } else {
        platform::run_app(init)
    }
}

pub(crate) fn shutdown_plugins(plugin_manager: &Arc<Mutex<PluginManager>>) {
    let mut manager = match plugin_manager.lock() {
        Ok(guard) => guard,
        Err(error) => {
            log::error!("Plugin manager lock poisoned during shutdown: {}", error);
            return;
        }
    };
    manager.shutdown();
}

impl TrayManager {
    pub fn new(
        feature_registry: Arc<FeatureRegistry>,
        shutdown_tx: broadcast::Sender<()>,
        shutdown_rx: broadcast::Receiver<()>,
        update_available: bool,
        events: Arc<EventBus>,
        post_pull_task: Option<tokio::task::JoinHandle<()>>,
    ) -> Self {
        if crate::headless_mode::active() {
            return Self {
                _tray: None,
                shutdown_tx,
                headless_shutdown_rx: Some(shutdown_rx),
                _post_pull_task: post_pull_task,
            };
        }
        let icon = icon::tray_icon(update_available, tone::current());
        let tray = match platform::create_tray(
            feature_registry,
            shutdown_tx.clone(),
            shutdown_rx,
            icon,
            update_available,
            events,
        ) {
            Ok(tray) => Some(tray),
            Err(error) => {
                report_missing_icon(&error);
                None
            }
        };
        Self {
            _tray: tray,
            shutdown_tx,
            headless_shutdown_rx: None,
            _post_pull_task: post_pull_task,
        }
    }

    pub(crate) fn wait_for_shutdown(&mut self) {
        if let Some(mut rx) = self.headless_shutdown_rx.take() {
            let _ = rx.blocking_recv();
        }
    }

    pub(crate) fn shutdown_sender(&self) -> broadcast::Sender<()> {
        self.shutdown_tx.clone()
    }
}

fn report_missing_icon(error: &anyhow::Error) {
    log::warn!("Running without the tray icon: {error:#}");
    let body = missing_icon_body(error);
    std::thread::spawn(move || {
        let _ = crate::settings_surface::wait_until_ready(MISSING_ICON_NOTICE_WAIT);
        crate::surfaces::show_plugin_notification(
            None,
            MISSING_ICON_TITLE,
            &body,
            NotificationLevel::Warn,
            None,
            None,
            None,
            None,
        );
    });
}

fn missing_icon_body(error: &anyhow::Error) -> String {
    format!("{error:#}\n\nPlugins and hotkeys keep working.")
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_missing_icon_notice_names_the_reason_and_what_still_works() {
        let error = anyhow::anyhow!(
            "Failed to create tray icon: Failed to load ayatana-appindicator3 or appindicator3 dynamic library"
        );

        assert_eq!(
            super::missing_icon_body(&error),
            "Failed to create tray icon: Failed to load ayatana-appindicator3 or appindicator3 dynamic library\n\nPlugins and hotkeys keep working."
        );
    }
}
