use crate::menu::router::EventRouter;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::broadcast;
use tray_icon::{TrayIconBuilder, TrayIcon, Icon, menu::{Menu, MenuEvent}};

pub fn create_tray(
    menu: Menu,
    router: EventRouter,
    shutdown_tx: broadcast::Sender<()>,
    icon: Icon,
) -> Result<TrayIcon> {
    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("QoL Tray")
        .with_icon(icon)
        .build()?;

    spawn_event_loop(router, shutdown_tx);

    Ok(tray_icon)
}

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
