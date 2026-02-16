use anyhow::Result;
use qol_tray::daemon::Daemon;
use qol_tray::features::{self, FeatureRegistry};
use qol_tray::hotkeys;
#[cfg(feature = "dev")]
use qol_tray::plugins::PluginLoader;
use qol_tray::plugins::PluginManager;
use qol_tray::tray::{self, TrayManager};
use qol_tray::updates;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

const DEFAULT_PORT: u16 = 42700;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if is_already_running() {
        eprintln!("qol-tray is already running on port {}", DEFAULT_PORT);
        show_already_running_notification();
        return Ok(());
    }

    log::info!("Starting QoL Tray daemon...");
    tray::platform::run_app(app_init)
}

fn is_already_running() -> bool {
    use std::net::{SocketAddr, TcpStream};
    let addr: SocketAddr = ([127, 0, 0, 1], DEFAULT_PORT).into();
    TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
}

fn show_already_running_notification() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("osascript")
            .args([
                "-e",
                "display notification \"Another instance is already running\" with title \"QoL Tray\"",
            ])
            .status();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("notify-send")
            .args(["QoL Tray", "Another instance is already running"])
            .status();
    }
}

fn app_init() -> Result<(TrayManager, Arc<Mutex<PluginManager>>)> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let (shutdown_tx, shutdown_rx, update_available, plugin_manager, feature_registry) =
        rt.block_on(async_init())?;

    std::thread::spawn(move || {
        rt.block_on(std::future::pending::<()>());
    });

    let tray = TrayManager::new(feature_registry, shutdown_tx, shutdown_rx, update_available)?;

    log::info!("QoL Tray daemon started successfully");
    Ok((tray, plugin_manager))
}

async fn async_init() -> Result<(
    broadcast::Sender<()>,
    broadcast::Receiver<()>,
    bool,
    Arc<Mutex<PluginManager>>,
    Arc<FeatureRegistry>,
)> {
    let update_available = check_for_updates().await;

    let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);

    let mut plugin_manager = PluginManager::new();
    plugin_manager.load_plugins()?;
    let plugin_manager = Arc::new(Mutex::new(plugin_manager));

    let daemon = Daemon::new();

    let mut feature_registry = FeatureRegistry::new();
    feature_registry.register(Box::new(features::plugin_store::Plugins::new()));
    let feature_registry = Arc::new(feature_registry);

    features::plugin_store::Plugins::start_server(plugin_manager.clone(), &daemon).await?;

    if let Err(e) = hotkeys::start_hotkey_listener(plugin_manager.clone()) {
        log::warn!("Failed to start hotkey listener: {}", e);
    }

    #[cfg(feature = "dev")]
    if let Ok(plugins_dir) = PluginLoader::default_plugin_dir() {
        daemon.start_discovery(plugins_dir);
    }

    Ok((
        shutdown_tx,
        shutdown_rx,
        update_available,
        plugin_manager,
        feature_registry,
    ))
}

async fn check_for_updates() -> bool {
    match tokio::time::timeout(Duration::from_secs(2), updates::check_for_updates()).await {
        Ok(Ok(has_update)) => has_update,
        Ok(Err(e)) => {
            log::debug!("Update check failed: {}", e);
            false
        }
        Err(_) => {
            log::debug!("Update check timed out");
            false
        }
    }
}
