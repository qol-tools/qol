use crate::features::FeatureRegistry;
use anyhow::Result;
use once_cell::sync::OnceCell;
use std::sync::Arc;
use tokio::sync::broadcast;
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

static QUIT_SIGNAL: OnceCell<std::sync::Condvar> = OnceCell::new();
static QUIT_MUTEX: OnceCell<std::sync::Mutex<bool>> = OnceCell::new();

pub fn create_tray(
    feature_registry: Arc<FeatureRegistry>,
    shutdown_tx: broadcast::Sender<()>,
    icon: Icon,
    update_available: bool,
) -> Result<TrayIcon> {
    QUIT_SIGNAL.get_or_init(std::sync::Condvar::new);
    QUIT_MUTEX.get_or_init(|| std::sync::Mutex::new(false));

    let (menu, router) = crate::menu::builder::build_menu(feature_registry, update_available)?;

    let tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("QoL Tray")
        .with_icon(icon)
        .build()?;

    super::spawn_menu_event_handler(shutdown_tx, router, signal_quit);

    Ok(tray_icon)
}

pub fn run_event_loop() {
    let mutex = QUIT_MUTEX.get().unwrap();
    let condvar = QUIT_SIGNAL.get().unwrap();

    let guard = mutex.lock().unwrap();
    let _ = condvar.wait_while(guard, |quit| !*quit);
}

fn signal_quit() {
    if let (Some(mutex), Some(condvar)) = (QUIT_MUTEX.get(), QUIT_SIGNAL.get()) {
        let mut quit = mutex.lock().unwrap();
        *quit = true;
        condvar.notify_all();
    }
}
