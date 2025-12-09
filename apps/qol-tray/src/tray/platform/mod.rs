#[cfg(target_os = "linux")]
mod linux;

use crate::features::FeatureRegistry;

#[cfg(not(target_os = "linux"))]
use crate::menu::router::EventRouter;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::broadcast;
use tray_icon::Icon;

#[cfg(not(target_os = "linux"))]
use tray_icon::{TrayIcon, TrayIconBuilder, menu::MenuEvent};

pub enum PlatformTray {
    #[cfg(target_os = "linux")]
    Linux,
    #[cfg(not(target_os = "linux"))]
    #[allow(dead_code)]
    Standard(TrayIcon),
}

#[cfg(target_os = "linux")]
pub fn create_tray(
    feature_registry: Arc<FeatureRegistry>,
    shutdown_tx: broadcast::Sender<()>,
    icon: Icon,
    update_available: bool,
) -> Result<PlatformTray> {
    linux::create_tray(feature_registry, shutdown_tx, icon, update_available)?;
    Ok(PlatformTray::Linux)
}

#[cfg(not(target_os = "linux"))]
pub fn create_tray(
    feature_registry: Arc<FeatureRegistry>,
    shutdown_tx: broadcast::Sender<()>,
    icon: Icon,
    update_available: bool,
) -> Result<PlatformTray> {
    let (menu, router) = crate::menu::builder::build_menu(feature_registry, update_available)?;

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("QoL Tray")
        .with_icon(icon)
        .build()?;

    spawn_event_loop(router, shutdown_tx);

    Ok(PlatformTray::Standard(tray_icon))
}

#[cfg(not(target_os = "linux"))]
fn spawn_event_loop(router: EventRouter, shutdown_tx: broadcast::Sender<()>) {
    let router = Arc::new(router);
    let menu_receiver = MenuEvent::receiver();

    std::thread::spawn(move || {
        while let Ok(event) = menu_receiver.recv() {
            if handle_event(&event.id.0, &router, &shutdown_tx) {
                break;
            }
        }
    });
}

#[cfg(not(target_os = "linux"))]
fn handle_event(
    event_id: &str,
    router: &EventRouter,
    shutdown_tx: &broadcast::Sender<()>,
) -> bool {
    log::debug!("Menu event: {}", event_id);

    let result = router.route(event_id);
    if let Err(e) = &result {
        log::error!("Error handling menu event: {}", e);
        return false;
    }

    if matches!(result, Ok(crate::menu::router::HandlerResult::Quit)) {
        log::info!("Quitting application");
        let _ = shutdown_tx.send(());
        return true;
    }

    false
}
