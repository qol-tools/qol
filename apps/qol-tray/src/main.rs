mod already_running_notification;

use anyhow::Result;
use qol_tray::daemon::Daemon;
use qol_tray::features::{self, FeatureRegistry};
use qol_tray::hotkeys;

use qol_tray::plugins::PluginManager;
use qol_tray::tray::{self, TrayManager};
use qol_tray::updates;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

const DEFAULT_PORT: u16 = 42700;

fn main() -> Result<()> {
    if let Some(code) = try_exec_subcommand() {
        std::process::exit(code);
    }

    #[cfg(feature = "dev")]
    let core_log_controls = qol_tray::logging::init_dev_logger();

    #[cfg(not(feature = "dev"))]
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    if is_already_running() {
        eprintln!("qol-tray is already running on port {}", DEFAULT_PORT);
        already_running_notification::show();
        return Ok(());
    }

    let startup_doctor = qol_tray::doctor::auto_fix_startup();
    println!(
        "[doctor] startup summary: attempted={}, applied={}, failures={}, ok={}, warn={}, error={}",
        startup_doctor.attempted,
        startup_doctor.applied,
        startup_doctor.failures.len(),
        startup_doctor.after.count_ok(),
        startup_doctor.after.count_warn(),
        startup_doctor.after.count_error()
    );

    log::info!("Starting QoL Tray daemon...");

    #[cfg(feature = "dev")]
    return tray::platform::run_app(move || app_init(core_log_controls));

    #[cfg(not(feature = "dev"))]
    tray::platform::run_app(app_init)
}

fn try_exec_subcommand() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) != Some("exec") {
        return None;
    }
    if args.len() == 4 && args[2] == "shortcut" {
        return Some(exec_shortcut(&args[3]));
    }
    if args.len() != 4 {
        eprintln!("Usage: qol-tray exec shortcut <shortcut_id>");
        eprintln!("       qol-tray exec <plugin_id> <action_id>");
        return Some(1);
    }
    let plugin_id = &args[2];
    let action_id = &args[3];
    if !qol_tray::plugins::manifest::is_valid_action_id(plugin_id) {
        eprintln!("Invalid plugin id: {}", plugin_id);
        return Some(1);
    }
    if !qol_tray::plugins::manifest::is_valid_action_id(action_id) {
        eprintln!("Invalid action id: {}", action_id);
        return Some(1);
    }
    Some(fire_action_request(plugin_id, action_id))
}

fn exec_shortcut(id: &str) -> i32 {
    if let Err(e) = qol_tray::shortcuts::validation::validate_id(id) {
        eprintln!("Invalid shortcut id: {}", e);
        return 1;
    }
    let config = match qol_tray::shortcuts::store::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load shortcuts: {}", e);
            return 1;
        }
    };
    let shortcut = match qol_tray::shortcuts::store::find_by_id(&config, id) {
        Some(s) => s,
        None => {
            eprintln!("Shortcut '{}' not found", id);
            return 1;
        }
    };
    if let Err(e) = qol_tray::shortcuts::executor::execute(&shortcut) {
        eprintln!("Failed to execute shortcut: {}", e);
        return 1;
    }
    0
}

fn fire_action_request(plugin_id: &str, action_id: &str) -> i32 {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};

    let addr: SocketAddr = ([127, 0, 0, 1], DEFAULT_PORT).into();
    let mut stream = match TcpStream::connect_timeout(&addr, Duration::from_secs(2)) {
        Ok(s) => s,
        Err(_) => {
            eprintln!("qol-tray is not running");
            return 1;
        }
    };
    let timeout = Some(Duration::from_secs(5));
    let _ = stream.set_read_timeout(timeout);
    let _ = stream.set_write_timeout(timeout);

    let request = format!(
        "POST /api/plugins/{}/actions/{} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        plugin_id, action_id, DEFAULT_PORT
    );
    if stream.write_all(request.as_bytes()).is_err() {
        eprintln!("Failed to send request");
        return 1;
    }

    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf);
    let response = String::from_utf8_lossy(&buf);

    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);

    if (200..300).contains(&status) {
        return 0;
    }

    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b)
        .unwrap_or("");
    let msg = if body.is_empty() {
        format!("Request failed (HTTP {})", status)
    } else {
        body.to_string()
    };
    eprintln!("{}", msg);
    1
}

fn is_already_running() -> bool {
    use std::net::{SocketAddr, TcpStream};
    let addr: SocketAddr = ([127, 0, 0, 1], DEFAULT_PORT).into();
    TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
}

struct InitResult {
    shutdown_tx: broadcast::Sender<()>,
    shutdown_rx: broadcast::Receiver<()>,
    update_available: bool,
    plugin_manager: Arc<Mutex<PluginManager>>,
    feature_registry: Arc<FeatureRegistry>,
    events: Arc<qol_tray::daemon::EventBus>,
}

#[cfg(feature = "dev")]
fn app_init(
    core_log_controls: qol_tray::logging::CoreControlsHandle,
) -> Result<(TrayManager, Arc<Mutex<PluginManager>>)> {
    app_init_inner(core_log_controls)
}

#[cfg(not(feature = "dev"))]
fn app_init() -> Result<(TrayManager, Arc<Mutex<PluginManager>>)> {
    app_init_inner()
}

fn app_init_inner(
    #[cfg(feature = "dev")] core_log_controls: qol_tray::logging::CoreControlsHandle,
) -> Result<(TrayManager, Arc<Mutex<PluginManager>>)> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let init = rt.block_on(async_init_inner(
        #[cfg(feature = "dev")]
        core_log_controls,
    ))?;
    std::thread::spawn(move || {
        rt.block_on(std::future::pending::<()>());
    });
    let tray = TrayManager::new(
        init.feature_registry,
        init.shutdown_tx,
        init.shutdown_rx,
        init.update_available,
        init.events,
    )?;
    log::info!("QoL Tray daemon started successfully");
    Ok((tray, init.plugin_manager))
}

async fn async_init_inner(
    #[cfg(feature = "dev")] core_log_controls: qol_tray::logging::CoreControlsHandle,
) -> Result<InitResult> {
    let update_available = check_for_updates().await;
    let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
    #[cfg(unix)]
    let _state_server = qol_tray::runtime::RuntimeServer::start();
    let mut plugin_manager = PluginManager::new();
    plugin_manager.load_plugins()?;
    let plugin_manager = Arc::new(Mutex::new(plugin_manager));
    let daemon = Daemon::new();
    let mut feature_registry = FeatureRegistry::new();
    feature_registry.register(Box::new(features::plugin_store::Plugins::new()));
    let feature_registry = Arc::new(feature_registry);
    features::plugin_store::Plugins::start_server(
        plugin_manager.clone(),
        &daemon,
        #[cfg(feature = "dev")]
        core_log_controls,
    )
    .await?;
    if let Err(e) = hotkeys::start_hotkey_listener(plugin_manager.clone()) {
        log::warn!("Failed to start hotkey listener: {}", e);
    }
    tokio::task::spawn_blocking(sync_launcher_apps);
    Ok(InitResult {
        shutdown_tx,
        shutdown_rx,
        update_available,
        plugin_manager,
        feature_registry,
        events: daemon.events.clone(),
    })
}

fn sync_launcher_apps() {
    features::launcher_apps::trigger_full_sync();
}

async fn check_for_updates() -> bool {
    if cfg!(feature = "dev") {
        return false;
    }
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
