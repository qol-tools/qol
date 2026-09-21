use crate::daemon::EventBus;
use crate::features::FeatureRegistry;
use crate::menu::router::EventRouter;
use crate::plugins::PluginManager;
use crate::tray::TrayManager;
use anyhow::Result;
use std::sync::{Arc, Mutex, OnceLock as OnceCell};
use tokio::sync::broadcast;
use tray_icon::menu::MenuEvent;
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

static QUIT_SIGNAL: OnceCell<std::sync::Condvar> = OnceCell::new();
static QUIT_MUTEX: OnceCell<std::sync::Mutex<bool>> = OnceCell::new();

pub enum PlatformTray {
    Windows { _tray_icon: TrayIcon },
}

pub(crate) fn request_shutdown(shutdown_tx: &broadcast::Sender<()>) {
    let _ = shutdown_tx.send(());
    signal_quit();
}

pub fn create_tray(
    feature_registry: Arc<FeatureRegistry>,
    shutdown_tx: broadcast::Sender<()>,
    shutdown_rx: broadcast::Receiver<()>,
    icon: Icon,
    update_available: bool,
    events: Arc<EventBus>,
) -> Result<PlatformTray> {
    let _ = shutdown_rx;
    let tray_icon = spawn_tray(
        feature_registry,
        shutdown_tx,
        icon,
        update_available,
        events,
    )?;
    Ok(PlatformTray::Windows {
        _tray_icon: tray_icon,
    })
}

pub fn run_app<F>(init: F) -> Result<()>
where
    F: FnOnce() -> Result<(TrayManager, Arc<Mutex<PluginManager>>)>,
{
    let (tray, plugin_manager) = init()?;
    let _signal_listener = crate::signal::install_signal_handler(tray.shutdown_sender())?;
    let _tray = tray;

    run_event_loop();

    shutdown_plugins(&plugin_manager);
    drop(plugin_manager);
    log::info!("Shutdown signal received, exiting...");
    Ok(())
}

fn spawn_tray(
    feature_registry: Arc<FeatureRegistry>,
    shutdown_tx: broadcast::Sender<()>,
    icon: Icon,
    update_available: bool,
    events: Arc<EventBus>,
) -> Result<TrayIcon> {
    QUIT_SIGNAL.get_or_init(std::sync::Condvar::new);
    QUIT_MUTEX.get_or_init(|| std::sync::Mutex::new(false));

    let (menu, router) =
        crate::menu::builder::build_menu(feature_registry, update_available, events)?;

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("QoL Tray")
        .with_icon(icon)
        .build()?;

    spawn_menu_event_handler(shutdown_tx, router, signal_quit);

    Ok(tray_icon)
}

fn run_event_loop() {
    let mutex = QUIT_MUTEX.get().unwrap();
    let condvar = QUIT_SIGNAL.get().unwrap();

    let guard = mutex.lock().unwrap();
    let _guard = condvar.wait_while(guard, |quit| !*quit);
}

fn signal_quit() {
    if let (Some(mutex), Some(condvar)) = (QUIT_MUTEX.get(), QUIT_SIGNAL.get()) {
        let mut quit = mutex.lock().unwrap();
        *quit = true;
        condvar.notify_all();
    }
}

fn spawn_menu_event_handler<F>(shutdown_tx: broadcast::Sender<()>, router: EventRouter, on_quit: F)
where
    F: FnOnce() + Send + 'static,
{
    let router = Arc::new(router);
    let menu_receiver = MenuEvent::receiver();
    std::thread::spawn(move || {
        while let Ok(event) = menu_receiver.recv() {
            log::debug!("Menu event: {}", event.id.0);
            if handle_menu_event(&router, &event.id.0, &shutdown_tx) {
                on_quit();
                break;
            }
        }
    });
}

fn handle_menu_event(
    router: &Arc<EventRouter>,
    event_id: &str,
    shutdown_tx: &broadcast::Sender<()>,
) -> bool {
    let result = router.route(event_id);
    if let Err(error) = &result {
        log::error!("Error handling menu event: {}", error);
        return false;
    }
    if !matches!(result, Ok(crate::menu::router::HandlerResult::Quit)) {
        return false;
    }
    log::info!("Quitting application");
    let _ = shutdown_tx.send(());
    true
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
