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
    url_scheme::install_delegate(&app, mtm);
    app.run();
}

/// Handles `qol://` URLs delivered by LaunchServices. When `CFBundleURLTypes`
/// is present in the bundle plist, AppKit installs the GetURL Apple-Event
/// handler and bridges it to `application:openURLs:`. Each URL is forwarded by
/// re-executing this binary as a pure courier (`URL_COURIER_FLAG` + the URL),
/// which `main`'s `try_url_courier` forwards to this running daemon and exits -
/// it never starts a second daemon, so there is no port-bind race with us. The
/// delegate is held weakly by `NSApp`, so the instance is leaked for process
/// life.
mod url_scheme {
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::{define_class, msg_send, MainThreadOnly};
    use objc2_app_kit::{NSApplication, NSApplicationDelegate};
    use objc2_foundation::{MainThreadMarker, NSArray, NSObject, NSObjectProtocol, NSURL};

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "QolUrlDelegate"]
        struct UrlDelegate;

        unsafe impl NSObjectProtocol for UrlDelegate {}

        unsafe impl NSApplicationDelegate for UrlDelegate {
            #[unsafe(method(application:openURLs:))]
            fn application_open_urls(&self, _app: &NSApplication, urls: &NSArray<NSURL>) {
                for i in 0..urls.count() {
                    let url = urls.objectAtIndex(i);
                    if let Some(s) = url.absoluteString() {
                        forward_url(&s.to_string());
                    }
                }
            }
        }
    );

    impl UrlDelegate {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            unsafe { msg_send![Self::alloc(mtm), init] }
        }
    }

    fn forward_url(url: &str) {
        if crate::commands::parse_qol_url(url).is_none() {
            return;
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Err(error) = std::process::Command::new(exe)
                .arg(crate::commands::URL_COURIER_FLAG)
                .arg(url)
                .spawn()
            {
                log::warn!("failed to spawn deep-link courier for {url}: {error}");
            }
        }
    }

    pub(super) fn install_delegate(app: &NSApplication, mtm: MainThreadMarker) {
        let delegate = UrlDelegate::new(mtm);
        app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // NSApp holds the delegate weakly; keep it alive for the process.
        std::mem::forget(delegate);
    }
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
