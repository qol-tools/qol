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
    if let Some(code) = try_handle_cli_flag() {
        std::process::exit(code);
    }
    if let Some(code) = try_exec_subcommand() {
        std::process::exit(code);
    }

    #[cfg(feature = "dev")]
    let core_log_controls = qol_tray::logging::init_logger();

    #[cfg(not(feature = "dev"))]
    qol_tray::logging::init_logger();

    if let Err(e) = qol_tray::installer::bootstrap_current_install() {
        log::error!("Failed to bootstrap current install: {}", e);
    }

    if is_already_running() {
        eprintln!("qol-tray is already running on port {}", DEFAULT_PORT);
        already_running_notification::show();
        return Ok(());
    }

    if let Err(e) = qol_tray::paths::init_runtime_dirs() {
        log::error!("Failed to initialize runtime directories: {}", e);
    }

    if let Ok(config_dir) = qol_tray::paths::shared_config_dir() {
        if let Err(e) = qol_tray::housekeeping::run_startup_cleanup(&config_dir) {
            log::error!("Migration failed: {}", e);
        }
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

fn try_handle_cli_flag() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    let flag = args.get(1).map(|s| s.as_str())?;
    match flag {
        "--version" | "-V" => {
            println!("qol-tray {}", qol_tray_version());
            Some(0)
        }
        "--help" | "-h" => {
            print_usage();
            Some(0)
        }
        _ => None,
    }
}

fn qol_tray_version() -> String {
    #[cfg(feature = "dev")]
    if let Some(override_version) = qol_tray::version::test_version_override() {
        return override_version.to_string();
    }
    env!("CARGO_PKG_VERSION").to_string()
}

fn print_usage() {
    println!("qol-tray {}", qol_tray_version());
    println!();
    println!("USAGE:");
    println!("    qol-tray                              Run the tray daemon");
    println!(
        "    qol-tray exec <plugin_id> <action>    Trigger a plugin action via the running daemon"
    );
    println!("    qol-tray exec shortcut <id>           Run a shortcut via the running daemon");
    println!("    qol-tray --version, -V                Print version and exit");
    println!("    qol-tray --help, -h                   Print this message and exit");
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
    if is_first_run() {
        std::thread::spawn(show_first_run_welcome);
    }
    Ok((tray, init.plugin_manager))
}

async fn async_init_inner(
    #[cfg(feature = "dev")] core_log_controls: qol_tray::logging::CoreControlsHandle,
) -> Result<InitResult> {
    let update_available = check_for_updates().await;
    let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
    #[cfg(unix)]
    let _state_server = qol_tray::runtime::RuntimeServer::start();
    let plugins_dir = qol_tray::plugins::PluginLoader::ensure_plugin_dir()?;
    let sync_service = Arc::new(qol_tray::sync::SyncService::new(plugins_dir)?);
    {
        let sync_for_pull = Arc::clone(&sync_service);
        tokio::spawn(async move {
            if let Err(error) = sync_for_pull.pull_on_launch().await {
                log::error!("Cloud profile pull on launch failed: {error:#}");
            }
        });
    }
    let mut plugin_manager = PluginManager::new();
    plugin_manager.load_plugins()?;
    let plugin_manager = Arc::new(Mutex::new(plugin_manager));
    {
        let startup_info = build_startup_info(&plugin_manager);
        qol_tray::logging::file_logger::log_startup(&startup_info);
    }
    let daemon = Daemon::new();
    let mut feature_registry = FeatureRegistry::new();
    feature_registry.register(Box::new(features::plugin_store::Plugins::new()));
    #[cfg(feature = "dev")]
    feature_registry.register(Box::new(features::mode_toggle::ModeToggle::new()));
    let feature_registry = Arc::new(feature_registry);
    features::plugin_store::Plugins::start_server(
        plugin_manager.clone(),
        &daemon,
        sync_service,
        #[cfg(feature = "dev")]
        core_log_controls,
    )
    .await?;
    match hotkeys::start_capture(plugin_manager.clone()) {
        Ok(()) => log::info!("Hotkey capture: kernel-level (evdev/uinput)"),
        Err(e) => {
            log::info!("Hotkey capture fallback to global_hotkey ({e})");
            if let Err(e) = hotkeys::start_hotkey_listener(plugin_manager.clone()) {
                log::warn!("Failed to start hotkey listener: {}", e);
            }
        }
    }
    qol_tray::plugins::daemon_supervisor::spawn_supervisor(
        plugin_manager.clone(),
        shutdown_tx.subscribe(),
    );
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

fn build_startup_info(
    pm: &std::sync::Arc<std::sync::Mutex<qol_tray::plugins::PluginManager>>,
) -> String {
    let plugins_desc = match pm.lock() {
        Ok(manager) => manager
            .plugins()
            .map(|p| {
                let id = &p.id;
                let version = &p.manifest.plugin.version;
                match p.manifest.build.commit.as_deref() {
                    Some(c) => format!("{}@{}@{}", id, version, c),
                    None => format!("{}@{}", id, version),
                }
            })
            .collect::<Vec<_>>()
            .join(", "),
        Err(_) => "unknown".to_string(),
    };

    let os_info = std::env::consts::OS;
    format!("{}, plugins: [{}]", os_info, plugins_desc)
}

fn first_run_marker_path() -> Option<std::path::PathBuf> {
    qol_tray::paths::shared_config_dir()
        .ok()
        .map(|d| d.join(".first-run-done"))
}

fn is_first_run() -> bool {
    first_run_marker_path()
        .map(|p| !p.exists())
        .unwrap_or(false)
}

fn show_first_run_welcome() {
    if let Some(path) = first_run_marker_path() {
        let _ = std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new("/")));
        let _ = std::fs::write(&path, "");
    }

    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("notify-send")
        .args([
            "--icon=qol-tray",
            "QoL Tray",
            "QoL Tray is running. Click the tray icon or visit http://localhost:42700 to get started.",
        ])
        .status();

    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("osascript")
        .args([
            "-e",
            "display notification \"QoL Tray is running. Click the menu bar icon to get started.\" with title \"QoL Tray\"",
        ])
        .status();

    std::thread::sleep(std::time::Duration::from_secs(1));

    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open")
        .arg("http://localhost:42700")
        .spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open")
        .arg("http://localhost:42700")
        .spawn();
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
