#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use crate::daemon::EventBus;
use crate::features::FeatureRegistry;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use crate::menu::router::EventRouter;
use crate::plugins::PluginManager;
use crate::tray::TrayManager;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
#[cfg(any(target_os = "macos", target_os = "windows"))]
use tray_icon::menu::MenuEvent;
use tray_icon::Icon;

#[cfg(any(target_os = "macos", target_os = "windows"))]
use tray_icon::TrayIcon;

pub enum PlatformTray {
    #[cfg(target_os = "linux")]
    Linux,
    #[cfg(target_os = "macos")]
    MacOS { _tray_icon: TrayIcon },
    #[cfg(target_os = "windows")]
    Windows { _tray_icon: TrayIcon },
}

pub fn create_tray(
    feature_registry: Arc<FeatureRegistry>,
    shutdown_tx: broadcast::Sender<()>,
    shutdown_rx: broadcast::Receiver<()>,
    icon: Icon,
    update_available: bool,
    events: Arc<EventBus>,
) -> Result<PlatformTray> {
    #[cfg(target_os = "linux")]
    {
        linux::store_shutdown_rx(shutdown_rx);
        linux::create_tray(feature_registry, shutdown_tx, icon, update_available, events)?;
        Ok(PlatformTray::Linux)
    }

    #[cfg(target_os = "macos")]
    {
        let _ = shutdown_rx;
        let tray_icon =
            macos::create_tray(feature_registry, shutdown_tx, icon, update_available, events)?;
        Ok(PlatformTray::MacOS {
            _tray_icon: tray_icon,
        })
    }

    #[cfg(target_os = "windows")]
    {
        let _ = shutdown_rx;
        let tray_icon =
            windows::create_tray(feature_registry, shutdown_tx, icon, update_available, events)?;
        Ok(PlatformTray::Windows {
            _tray_icon: tray_icon,
        })
    }
}

/// Run the application. Calls `init` to create the tray, then blocks until shutdown.
pub fn run_app<F>(init: F) -> Result<()>
where
    F: FnOnce() -> Result<(TrayManager, Arc<Mutex<PluginManager>>)>,
{
    let (_tray, plugin_manager) = init()?;

    #[cfg(target_os = "linux")]
    linux::run_event_loop();

    #[cfg(target_os = "macos")]
    macos::run_event_loop();

    #[cfg(target_os = "windows")]
    windows::run_event_loop();

    shutdown_plugins(&plugin_manager);
    drop(plugin_manager);
    log::info!("Shutdown signal received, exiting...");
    Ok(())
}

fn shutdown_plugins(plugin_manager: &Arc<Mutex<PluginManager>>) {
    let mut manager = match plugin_manager.lock() {
        Ok(guard) => guard,
        Err(error) => {
            log::error!("Plugin manager lock poisoned during shutdown: {}", error);
            return;
        }
    };
    manager.shutdown();
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn spawn_menu_event_handler<F>(
    shutdown_tx: broadcast::Sender<()>,
    router: EventRouter,
    on_quit: F,
) where
    F: FnOnce() + Send + 'static,
{
    let router = Arc::new(router);
    let menu_receiver = MenuEvent::receiver();

    std::thread::spawn(move || {
        while let Ok(event) = menu_receiver.recv() {
            log::debug!("Menu event: {}", event.id.0);

            let result = router.route(&event.id.0);
            if let Err(e) = &result {
                log::error!("Error handling menu event: {}", e);
                continue;
            }

            if matches!(result, Ok(crate::menu::router::HandlerResult::Quit)) {
                log::info!("Quitting application");
                let _ = shutdown_tx.send(());
                on_quit();
                break;
            }
        }
    });
}
