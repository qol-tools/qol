use crate::plugins::PluginManager;
use crate::features::FeatureRegistry;
use anyhow::Result;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tray_icon::{TrayIcon, TrayIconBuilder, Icon};
use gtk::{self, glib};
use once_cell::sync::OnceCell;
use std::sync::mpsc;

static PLUGIN_REFRESH_TX: OnceCell<mpsc::Sender<()>> = OnceCell::new();

pub fn request_plugin_refresh() {
    if let Some(tx) = PLUGIN_REFRESH_TX.get() {
        let _ = tx.send(());
        log::info!("Plugin refresh requested");
    }
}

pub fn create_tray(
    plugin_manager: Arc<Mutex<PluginManager>>,
    feature_registry: Arc<FeatureRegistry>,
    shutdown_tx: broadcast::Sender<()>,
    icon: Icon,
) -> Result<()> {
    let (refresh_tx, refresh_rx) = mpsc::channel::<()>();
    let _ = PLUGIN_REFRESH_TX.set(refresh_tx);

    std::thread::spawn(move || {
        if gtk::init().is_err() {
            log::error!("Failed to initialize GTK");
            return;
        }

        let (menu, router) = match crate::menu::builder::build_menu(
            plugin_manager.clone(),
            feature_registry.clone(),
        ) {
            Ok(result) => result,
            Err(e) => {
                log::error!("Failed to build menu: {}", e);
                return;
            }
        };

        let tray_icon = match TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("QoL Tray")
            .with_icon(icon.clone())
            .build()
        {
            Ok(ti) => ti,
            Err(e) => {
                log::error!("Failed to create tray icon: {}", e);
                return;
            }
        };

        let tray_icon = Arc::new(Mutex::new(tray_icon));

        setup_event_loop(
            router,
            shutdown_tx,
            refresh_rx,
            tray_icon,
            plugin_manager,
            feature_registry,
        );

        gtk::main();
    });

    Ok(())
}

fn setup_event_loop(
    router: crate::menu::router::EventRouter,
    shutdown_tx: broadcast::Sender<()>,
    refresh_rx: mpsc::Receiver<()>,
    tray_icon: Arc<Mutex<TrayIcon>>,
    plugin_manager: Arc<Mutex<PluginManager>>,
    feature_registry: Arc<FeatureRegistry>,
) {
    use tray_icon::menu::MenuEvent;

    let menu_receiver = MenuEvent::receiver();
    let router = Arc::new(Mutex::new(router));

    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        // Check for plugin refresh requests
        if refresh_rx.try_recv().is_ok() {
            log::info!("Refreshing plugins and menu...");

            // Reload plugins
            {
                let mut manager = plugin_manager.lock().unwrap();
                if let Err(e) = manager.reload_plugins() {
                    log::error!("Failed to reload plugins: {}", e);
                }
            }

            // Rebuild menu
            match crate::menu::builder::build_menu(
                plugin_manager.clone(),
                feature_registry.clone(),
            ) {
                Ok((new_menu, new_router)) => {
                    if let Ok(tray) = tray_icon.lock() {
                        tray.set_menu(Some(Box::new(new_menu)));
                        log::info!("Menu updated successfully");
                    }
                    *router.lock().unwrap() = new_router;
                }
                Err(e) => {
                    log::error!("Failed to rebuild menu: {}", e);
                }
            }
        }

        // Handle menu events
        while let Ok(event) = menu_receiver.try_recv() {
            let event_id = event.id.0.clone();
            log::debug!("Menu event: {}", event_id);

            match router.lock().unwrap().route(&event_id) {
                Ok(crate::menu::router::HandlerResult::Quit) => {
                    log::info!("Quitting application");
                    gtk::main_quit();
                    let _ = shutdown_tx.send(());
                    return glib::ControlFlow::Break;
                }
                Ok(_) => {}
                Err(e) => {
                    log::error!("Error handling menu event: {}", e);
                }
            }
        }

        glib::ControlFlow::Continue
    });
}
