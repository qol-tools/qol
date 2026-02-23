use crate::daemon::EventBus;
use crate::features::FeatureRegistry;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::broadcast;
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub fn create_tray(
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

pub fn run_event_loop() {
    use objc2_app_kit::NSApplication;
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.run();
}

fn stop_event_loop() {
    use objc2_app_kit::NSApplication;
    use objc2_foundation::MainThreadMarker;

    let mtm = MainThreadMarker::new().expect("must be on main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.terminate(None);
}
