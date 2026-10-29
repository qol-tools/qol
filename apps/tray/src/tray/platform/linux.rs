use crate::daemon::EventBus;
use crate::features::FeatureRegistry;
use crate::plugins::PluginManager;
use crate::tray::TrayManager;
use anyhow::Result;
use gtk::{self, glib};
use std::sync::{Arc, Mutex, OnceLock as OnceCell};
use tokio::sync::broadcast;
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

type StartupResult = std::result::Result<(), String>;
type StartupTx = std::sync::mpsc::Sender<StartupResult>;

static SHUTDOWN_RX: OnceCell<std::sync::Mutex<Option<broadcast::Receiver<()>>>> = OnceCell::new();

pub(crate) const ICON_SIZE: u32 = 32;
pub(crate) const ICON_SIDE_PADDING: u32 = 0;
const TONE_REFRESH_SECONDS: u32 = 10;
const SLOW_STARTUP_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

pub enum PlatformTray {
    Linux,
}

pub(crate) fn request_shutdown(shutdown_tx: &broadcast::Sender<()>) {
    let _ = shutdown_tx.send(());
}

pub fn create_tray(
    feature_registry: Arc<FeatureRegistry>,
    shutdown_tx: broadcast::Sender<()>,
    shutdown_rx: broadcast::Receiver<()>,
    icon: Icon,
    update_available: bool,
    events: Arc<EventBus>,
) -> Result<PlatformTray> {
    store_shutdown_rx(shutdown_rx);
    spawn_tray(
        feature_registry,
        shutdown_tx,
        icon,
        update_available,
        events,
    )?;
    Ok(PlatformTray::Linux)
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

fn store_shutdown_rx(rx: broadcast::Receiver<()>) {
    let _ = SHUTDOWN_RX.set(std::sync::Mutex::new(Some(rx)));
}

fn run_event_loop() {
    if let Some(mutex) = SHUTDOWN_RX.get() {
        if let Some(mut rx) = mutex.lock().unwrap().take() {
            let _ = rx.blocking_recv();
        }
    }
}

fn spawn_tray(
    feature_registry: Arc<FeatureRegistry>,
    shutdown_tx: broadcast::Sender<()>,
    icon: Icon,
    update_available: bool,
    events: Arc<EventBus>,
) -> Result<()> {
    let (startup_tx, startup_rx) = std::sync::mpsc::channel::<StartupResult>();
    std::thread::spawn(move || {
        run_tray_thread(
            startup_tx,
            feature_registry,
            shutdown_tx,
            icon,
            update_available,
            events,
        )
    });
    match startup_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            log::warn!(
                "The Linux tray icon is slow to start, so qol-tray keeps starting without it"
            );
            std::thread::spawn(move || {
                follow_slow_startup(startup_rx, crate::tray::report_missing_icon)
            });
            Ok(())
        }
        received => startup_result(received),
    }
}

fn follow_slow_startup(
    startup_rx: std::sync::mpsc::Receiver<StartupResult>,
    report: impl FnOnce(&anyhow::Error),
) {
    match startup_result(startup_rx.recv_timeout(SLOW_STARTUP_WAIT)) {
        Ok(()) => log::info!("The Linux tray icon started late"),
        Err(error) => report(&error),
    }
}

fn startup_result(
    received: std::result::Result<StartupResult, std::sync::mpsc::RecvTimeoutError>,
) -> Result<()> {
    match received {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => anyhow::bail!(message),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            anyhow::bail!("Timed out while initializing Linux tray")
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            anyhow::bail!("The Linux tray thread stopped before the icon was ready")
        }
    }
}

fn run_tray_thread(
    startup_tx: StartupTx,
    feature_registry: Arc<FeatureRegistry>,
    shutdown_tx: broadcast::Sender<()>,
    icon: Icon,
    update_available: bool,
    events: Arc<EventBus>,
) {
    if gtk::init().is_err() {
        let _ = startup_tx.send(Err("Failed to initialize GTK".to_string()));
        return;
    }
    let Some((tray_icon, router)) = build_menu_and_icon(
        &startup_tx,
        feature_registry,
        update_available,
        events,
        icon,
    ) else {
        return;
    };
    setup_event_loop(router, shutdown_tx);
    follow_panel_tone(tray_icon, update_available);
    let _ = startup_tx.send(Ok(()));
    gtk::main();
}

fn follow_panel_tone(tray_icon: TrayIcon, update_available: bool) {
    use crate::tray::{icon, tone};

    let mut shown = tone::current();
    glib::timeout_add_seconds_local(TONE_REFRESH_SECONDS, move || {
        let current = tone::current();
        if current != shown {
            shown = current;
            let art = icon::tray_icon(update_available, current);
            if let Err(error) = tray_icon.set_icon(Some(art)) {
                log::warn!("failed to swap the tray icon for the panel tone: {error}");
            }
        }
        glib::ControlFlow::Continue
    });
}

fn build_menu_and_icon(
    startup_tx: &StartupTx,
    feature_registry: Arc<FeatureRegistry>,
    update_available: bool,
    events: Arc<EventBus>,
    icon: Icon,
) -> Option<(TrayIcon, crate::menu::router::EventRouter)> {
    let (menu, router) =
        match crate::menu::builder::build_menu(feature_registry, update_available, events) {
            Ok(r) => r,
            Err(e) => {
                let _ = startup_tx.send(Err(format!("Failed to build menu: {}", e)));
                return None;
            }
        };
    let built = catch_tray_panic(move || {
        TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("QoL Tray")
            .with_icon(icon)
            .build()
            .map_err(|e| e.to_string())
    });
    match built {
        Ok(t) => Some((t, router)),
        Err(e) => {
            let _ = startup_tx.send(Err(format!("Failed to create tray icon: {}", e)));
            None
        }
    }
}

fn catch_tray_panic<T>(build: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(build))
        .unwrap_or_else(|panic| Err(crate::doctor::panic_payload_message(&panic)))
}

fn setup_event_loop(router: crate::menu::router::EventRouter, shutdown_tx: broadcast::Sender<()>) {
    use tray_icon::menu::MenuEvent;

    let menu_receiver = MenuEvent::receiver();
    let router = Arc::new(router);

    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        process_pending_events(menu_receiver, &router, &shutdown_tx)
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
    if !should_quit {
        return false;
    }

    log::info!("Quitting application");
    gtk::main_quit();
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

#[cfg(test)]
mod tests {
    #[test]
    fn request_shutdown_broadcasts() {
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::broadcast::channel(1);
        super::request_shutdown(&shutdown_tx);

        assert!(shutdown_rx.try_recv().is_ok());
    }

    #[test]
    fn a_tray_library_that_panics_becomes_a_startup_error() {
        let missing_library = super::catch_tray_panic::<()>(|| {
            panic!("Failed to load ayatana-appindicator3 or appindicator3 dynamic library")
        });
        let refused = super::catch_tray_panic::<()>(|| Err("no status notifier".to_string()));
        let built = super::catch_tray_panic(|| Ok(7));

        assert_eq!(
            missing_library,
            Err(
                "Failed to load ayatana-appindicator3 or appindicator3 dynamic library".to_string()
            )
        );
        assert_eq!(refused, Err("no status notifier".to_string()));
        assert_eq!(built, Ok(7));
    }

    #[test]
    fn a_slow_tray_start_reports_the_icon_missing_only_if_it_fails() {
        let follow = |result: Option<super::StartupResult>| {
            let (startup_tx, startup_rx) = std::sync::mpsc::channel();
            if let Some(result) = result {
                startup_tx.send(result).unwrap();
            }
            drop(startup_tx);
            let mut reported = None;
            super::follow_slow_startup(startup_rx, |error| reported = Some(format!("{error:#}")));
            reported
        };

        assert_eq!(follow(Some(Ok(()))), None);
        assert_eq!(
            follow(Some(Err("no status notifier".to_string()))),
            Some("no status notifier".to_string())
        );
        assert_eq!(
            follow(None),
            Some("The Linux tray thread stopped before the icon was ready".to_string())
        );
    }
}
