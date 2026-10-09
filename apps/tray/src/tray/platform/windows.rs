use crate::daemon::EventBus;
use crate::features::FeatureRegistry;
use crate::menu::router::EventRouter;
use crate::plugins::PluginManager;
use crate::tray::TrayManager;
use anyhow::Result;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;
use tray_icon::menu::MenuEvent;
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostThreadMessageW, TranslateMessage, MSG, WM_QUIT,
};

static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);
static PUMP_THREAD: AtomicU32 = AtomicU32::new(0);

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
    init_process();
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

fn init_process() {
    if unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } == 0 {
        log::debug!("per-monitor DPI awareness was already set or is unavailable");
    }
}

fn run_event_loop() {
    PUMP_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
    let mut message: MSG = unsafe { std::mem::zeroed() };
    while !QUIT_REQUESTED.load(Ordering::SeqCst) {
        match unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } {
            0 => break,
            -1 => {
                log::error!("GetMessageW failed: {}", std::io::Error::last_os_error());
                break;
            }
            _ => unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            },
        }
    }
}

fn signal_quit() {
    QUIT_REQUESTED.store(true, Ordering::SeqCst);
    let pump_thread = PUMP_THREAD.load(Ordering::SeqCst);
    if pump_thread != 0 {
        unsafe { PostThreadMessageW(pump_thread, WM_QUIT, 0, 0) };
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
