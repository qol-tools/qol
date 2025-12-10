use crate::features::FeatureRegistry;
use anyhow::Result;
use gtk::{self, glib};
use once_cell::sync::OnceCell;
use std::sync::Arc;
use tokio::sync::broadcast;
use tray_icon::{Icon, TrayIconBuilder};

static SHUTDOWN_RX: OnceCell<std::sync::Mutex<Option<broadcast::Receiver<()>>>> = OnceCell::new();

pub fn store_shutdown_rx(rx: broadcast::Receiver<()>) {
    let _ = SHUTDOWN_RX.set(std::sync::Mutex::new(Some(rx)));
}

pub fn run_event_loop() {
    if let Some(mutex) = SHUTDOWN_RX.get() {
        if let Some(mut rx) = mutex.lock().unwrap().take() {
            let _ = rx.blocking_recv();
        }
    }
}

pub fn create_tray(
    feature_registry: Arc<FeatureRegistry>,
    shutdown_tx: broadcast::Sender<()>,
    icon: Icon,
    update_available: bool,
) -> Result<()> {
    std::thread::spawn(move || {
        if gtk::init().is_err() {
            log::error!("Failed to initialize GTK");
            return;
        }

        let (menu, router) = match crate::menu::builder::build_menu(feature_registry, update_available) {
            Ok(result) => result,
            Err(e) => {
                log::error!("Failed to build menu: {}", e);
                return;
            }
        };

        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("QoL Tray")
            .with_icon(icon)
            .build();

        let tray_icon = match tray_icon {
            Ok(icon) => icon,
            Err(e) => {
                log::error!("Failed to create tray icon: {}", e);
                return;
            }
        };

        setup_event_loop(router, shutdown_tx);
        std::mem::forget(tray_icon);
        gtk::main();
    });

    Ok(())
}

fn setup_event_loop(router: crate::menu::router::EventRouter, shutdown_tx: broadcast::Sender<()>) {
    use tray_icon::menu::MenuEvent;

    let menu_receiver = MenuEvent::receiver();
    let router = Arc::new(router);

    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        process_pending_events(&menu_receiver, &router, &shutdown_tx)
    });
}

fn process_pending_events(
    receiver: &tray_icon::menu::MenuEventReceiver,
    router: &crate::menu::router::EventRouter,
    shutdown_tx: &broadcast::Sender<()>,
) -> glib::ControlFlow {
    while let Ok(event) = receiver.try_recv() {
        if handle_menu_event(&event.id.0, router, shutdown_tx) {
            return glib::ControlFlow::Break;
        }
    }
    glib::ControlFlow::Continue
}

fn handle_menu_event(
    event_id: &str,
    router: &crate::menu::router::EventRouter,
    shutdown_tx: &broadcast::Sender<()>,
) -> bool {
    log::debug!("Menu event: {}", event_id);

    let result = router.route(event_id);
    if let Err(e) = &result {
        log::error!("Error handling menu event: {}", e);
        return false;
    }

    let should_quit = matches!(result, Ok(crate::menu::router::HandlerResult::Quit));
    if !should_quit { return false; }

    log::info!("Quitting application");
    gtk::main_quit();
    let _ = shutdown_tx.send(());
    true
}
