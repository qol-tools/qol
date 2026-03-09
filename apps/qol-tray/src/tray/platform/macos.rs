use crate::daemon::EventBus;
use crate::features::FeatureRegistry;
use anyhow::Result;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::broadcast;
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

type ShutdownFn = Box<dyn FnOnce() + Send>;
static SHUTDOWN_FN: OnceLock<Mutex<Option<ShutdownFn>>> = OnceLock::new();

pub(super) fn register_shutdown_fn(f: impl FnOnce() + Send + 'static) {
    let cell = SHUTDOWN_FN.get_or_init(|| Mutex::new(None));
    if let Ok(mut guard) = cell.lock() {
        *guard = Some(Box::new(f));
    }
}

pub(super) fn create_tray(
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

    super::spawn_menu_event_handler(shutdown_tx, router, stop_event_loop);

    Ok(tray_icon)
}

pub(super) fn run_event_loop() {
    use objc2_app_kit::NSApplication;
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.run();
}

fn stop_event_loop() {
    use objc2_app_kit::NSApplication;
    use objc2_foundation::MainThreadMarker;

    extern "C" {
        static _dispatch_main_q: std::ffi::c_void;
        fn dispatch_async_f(
            queue: *const std::ffi::c_void,
            context: *mut std::ffi::c_void,
            work: extern "C" fn(*mut std::ffi::c_void),
        );
    }

    extern "C" fn terminate_on_main(_: *mut std::ffi::c_void) {
        if let Some(cell) = SHUTDOWN_FN.get() {
            if let Ok(mut guard) = cell.lock() {
                if let Some(f) = guard.take() {
                    f();
                }
            }
        }
        let mtm = MainThreadMarker::new().unwrap();
        let app = NSApplication::sharedApplication(mtm);
        app.terminate(None);
    }

    unsafe {
        dispatch_async_f(&_dispatch_main_q, std::ptr::null_mut(), terminate_on_main);
    }
}
