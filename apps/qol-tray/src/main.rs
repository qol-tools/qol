mod already_running_notification;

use anyhow::Result;
use qol_tray::daemon::Daemon;

#[cfg(feature = "dev")]
static PENDING_HEAL_REPORT: std::sync::OnceLock<qol_tray::dev::boot_contract::HealReport> =
    std::sync::OnceLock::new();
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
    if let Some(code) = try_open_subcommand() {
        std::process::exit(code);
    }
    if let Some(code) = try_url_courier() {
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
        match qol_migrations::run_pre_flight(&config_dir, env!("CARGO_PKG_VERSION")) {
            Ok(reports) if reports.is_empty() => {}
            Ok(reports) => {
                for report in reports {
                    log::info!(
                        "qol-migrations[pre-flight]: applied {} (archived {} paths)",
                        report.name,
                        report.archived.len()
                    );
                }
            }
            Err(error) => log::error!("qol-migrations[pre-flight] failed: {error:#}"),
        }
        if let Err(e) = qol_tray::housekeeping::run_startup_cleanup(&config_dir) {
            log::error!("Housekeeping failed: {}", e);
        }
    }

    #[cfg(feature = "dev")]
    {
        if let Ok(config_dir) = qol_tray::paths::shared_config_dir() {
            let env = qol_tray::installer::boot_environment::default_boot_environment();
            let lister = qol_tray::dev::boot_contract::GitWorktreeLister;
            let probe = qol_tray::dev::boot_contract::FsBinaryProbe;
            let report = qol_tray::dev::boot_contract::heal_drift_on_startup(
                env.as_ref(),
                &config_dir,
                &lister,
                &probe,
            );
            for event in &report.events {
                log::warn!("[boot-contract] drift observed: {:?}", event);
            }
            for action in &report.actions {
                log::info!("[boot-contract] applied: {:?}", action);
            }
            for failure in &report.failures {
                log::error!("[boot-contract] failed: {:?}", failure);
            }
            if !report.events.is_empty() {
                let _ = PENDING_HEAL_REPORT.set(report);
            }
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
        s if s.starts_with("--write-mode=") => {
            let exit = write_mode_flag(&s["--write-mode=".len()..]);
            if exit != 0 {
                Some(exit)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn write_mode_flag(value: &str) -> i32 {
    let mode = match qol_tray::mode::ModeFlag::parse_cli(value) {
        Ok(m) => m,
        Err(msg) => {
            eprintln!("{}", msg);
            return 1;
        }
    };
    if let Err(e) = qol_tray::mode::ModeConfig::set(mode) {
        eprintln!("Failed to write mode.json: {}", e);
        return 1;
    }
    println!("mode.json set to {:?}", mode);
    0
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
    println!(
        "    qol-tray open <route>                 Open the app at an in-app route (e.g. shortcuts/add)"
    );
    println!("    qol-tray --write-mode=<dev|prod>      Write mode.json then run the tray");
    println!("    qol-tray --version, -V                Print version and exit");
    println!("    qol-tray --help, -h                   Print this message and exit");
}

fn try_open_subcommand() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) != Some("open") {
        return None;
    }
    let Some(route) = args.get(2) else {
        eprintln!("Usage: qol-tray open <route>   e.g. qol-tray open shortcuts/add");
        return Some(1);
    };
    Some(forward_route(route))
}

/// Route stashed when a bare `qol://` URL arrives in argv on a Linux cold launch
/// (daemon not yet running); opened once the server is listening. macOS cold
/// launches do not use this - the daemon process never sees the URL in argv;
/// it arrives post-launch via the `openURLs` delegate, which spawns a separate
/// courier process. See `try_url_courier`.
static PENDING_COLD_ROUTE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Navigate an already-open UI tab to `route`, falling back to opening a fresh
/// browser tab. Shared by `qol-tray open` and the `qol://` courier.
fn forward_route(route: &str) -> i32 {
    if navigated_open_tab(route) {
        return 0;
    }
    let url = qol_tray::commands::deeplink_url(route, DEFAULT_PORT);
    match qol_tray::paths::open_url(&url) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Failed to open {url}: {e}");
            1
        }
    }
}

/// How this invocation carries a `qol://` URL, decided purely from argv.
#[derive(Debug, PartialEq, Eq)]
enum UrlInvocation {
    /// macOS `openURLs` delegate re-exec (`URL_COURIER_FLAG` + URL): forward the
    /// route (when valid) to the running daemon and exit. NEVER starts a daemon.
    Courier(Option<String>),
    /// Bare `qol://` URL in argv (Linux `.desktop %u`, or any direct invocation):
    /// forward if a daemon is running, otherwise become the daemon for it.
    Argv(String),
    /// Not a `qol://` invocation; normal startup continues.
    NotUrl,
}

fn classify_url_args(args: &[String]) -> UrlInvocation {
    if args.get(1).map(String::as_str) == Some(qol_tray::commands::URL_COURIER_FLAG) {
        let route = args
            .get(2)
            .and_then(|u| qol_tray::commands::parse_qol_url(u));
        return UrlInvocation::Courier(route);
    }
    match args
        .get(1)
        .and_then(|u| qol_tray::commands::parse_qol_url(u))
    {
        Some(route) => UrlInvocation::Argv(route),
        None => UrlInvocation::NotUrl,
    }
}

/// Handle a `qol://<route>` URL. Linux `.desktop %u` passes it bare in argv; the
/// macOS `openURLs` delegate re-execs us as a courier (`URL_COURIER_FLAG`). A
/// courier always forwards-then-exits so it can never race the parent daemon for
/// the port. A bare argv URL forwards to a running daemon, or - on a cold launch
/// where this process becomes the daemon - stashes the route for `app_init_inner`
/// to open once the server is listening.
fn try_url_courier() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    match classify_url_args(&args) {
        UrlInvocation::Courier(Some(route)) => Some(courier_forward_with_retry(&route)),
        UrlInvocation::Courier(None) => Some(1),
        UrlInvocation::Argv(route) => {
            if is_already_running() {
                Some(forward_route(&route))
            } else {
                let _ = PENDING_COLD_ROUTE.set(route);
                None
            }
        }
        UrlInvocation::NotUrl => None,
    }
}

/// A macOS courier is spawned by the running daemon's URL delegate, but that
/// daemon's HTTP server may still be binding. Wait briefly (up to ~2s) for it to
/// accept connections so we navigate the live tab instead of opening a dead one,
/// then forward.
fn courier_forward_with_retry(route: &str) -> i32 {
    for _ in 0..40 {
        if is_already_running() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    forward_route(route)
}

fn open_pending_cold_route(route: &str) {
    // Let the HTTP server bind before pointing a browser at the route.
    std::thread::sleep(Duration::from_millis(1500));
    let url = qol_tray::commands::deeplink_url(route, DEFAULT_PORT);
    let _ = qol_tray::paths::open_url(&url);
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

/// Send a POST to the running daemon over loopback. Returns the HTTP status
/// code and the response body. The `Origin` header is set to the loopback UI
/// origin so the request passes `reject_cross_site_mutations`.
fn post_to_daemon(path: &str, body: &str) -> std::io::Result<(u16, String)> {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};

    let addr: SocketAddr = ([127, 0, 0, 1], DEFAULT_PORT).into();
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    let timeout = Some(Duration::from_secs(5));
    let _ = stream.set_read_timeout(timeout);
    let _ = stream.set_write_timeout(timeout);

    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
        port = DEFAULT_PORT,
        len = body.len(),
    );
    stream.write_all(request.as_bytes())?;

    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf);
    let response = String::from_utf8_lossy(&buf);
    let status = response
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    Ok((status, body))
}

/// Ask the running daemon to navigate an already-open UI tab to `route`.
/// Returns true only when a tab was subscribed and the event was delivered;
/// any connection error or `delivered:false` returns false so the caller
/// falls back to opening a fresh browser tab.
fn navigated_open_tab(route: &str) -> bool {
    let body = serde_json::json!({ "route": route }).to_string();
    match post_to_daemon("/api/navigate", &body) {
        Ok((status, body)) if (200..300).contains(&status) => {
            serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("delivered").and_then(|d| d.as_bool()))
                .unwrap_or(false)
        }
        _ => false,
    }
}

fn fire_action_request(plugin_id: &str, action_id: &str) -> i32 {
    let path = format!("/api/plugins/{plugin_id}/actions/{action_id}");
    match post_to_daemon(&path, "") {
        Ok((status, _)) if (200..300).contains(&status) => 0,
        Ok((status, body)) => {
            let msg = if body.is_empty() {
                format!("Request failed (HTTP {})", status)
            } else {
                body
            };
            eprintln!("{}", msg);
            1
        }
        Err(_) => {
            eprintln!("qol-tray is not running");
            1
        }
    }
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
    if let Some(route) = PENDING_COLD_ROUTE.get() {
        let route = route.clone();
        std::thread::spawn(move || open_pending_cold_route(&route));
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
    if let Ok(config_dir) = qol_tray::paths::shared_config_dir() {
        if let Err(error) = qol_tray::migrations_startup::run_post_auth_if_authed(&config_dir).await
        {
            log::error!("qol-migrations[post-auth] failed: {error:#}");
        }
    }
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
    #[cfg(feature = "dev")]
    if let Some(report) = PENDING_HEAL_REPORT.get() {
        daemon
            .events
            .send(qol_tray::daemon::DaemonEvent::BootTargetHealed {
                report: report.clone(),
            });
    }
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
            } else {
                tokio::spawn(async {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    hotkeys::trigger_reload();
                });
            }
        }
    }
    qol_tray::plugins::daemon_supervisor::spawn_supervisor(
        plugin_manager.clone(),
        shutdown_tx.subscribe(),
    );
    {
        let plugin_manager = plugin_manager.clone();
        tokio::task::spawn_blocking(move || {
            if let Ok(mut manager) = plugin_manager.lock() {
                manager.autostart_daemons();
            } else {
                log::error!("plugin manager lock poisoned during daemon autostart");
            }
        });
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

#[cfg(test)]
mod url_courier_tests {
    use super::{classify_url_args, UrlInvocation};
    use qol_tray::commands::URL_COURIER_FLAG;

    fn args(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn courier_flag_with_valid_url_is_courier_with_route() {
        let a = args(&["qol-tray", URL_COURIER_FLAG, "qol://shortcuts/add?type=url"]);
        assert_eq!(
            classify_url_args(&a),
            UrlInvocation::Courier(Some("shortcuts/add?type=url".to_string()))
        );
    }

    #[test]
    fn courier_flag_with_bad_url_stays_courier_so_it_never_starts_a_daemon() {
        let a = args(&["qol-tray", URL_COURIER_FLAG, "https://evil.example"]);
        assert_eq!(classify_url_args(&a), UrlInvocation::Courier(None));
        let missing = args(&["qol-tray", URL_COURIER_FLAG]);
        assert_eq!(classify_url_args(&missing), UrlInvocation::Courier(None));
    }

    #[test]
    fn bare_qol_url_in_argv_is_argv_route() {
        let a = args(&["qol-tray", "qol://shortcuts"]);
        assert_eq!(
            classify_url_args(&a),
            UrlInvocation::Argv("shortcuts".to_string())
        );
    }

    #[test]
    fn non_url_invocations_are_not_url() {
        assert_eq!(
            classify_url_args(&args(&["qol-tray"])),
            UrlInvocation::NotUrl
        );
        assert_eq!(
            classify_url_args(&args(&["qol-tray", "exec", "p", "a"])),
            UrlInvocation::NotUrl
        );
        assert_eq!(
            classify_url_args(&args(&["qol-tray", "open", "shortcuts"])),
            UrlInvocation::NotUrl
        );
    }
}
