mod host_cli;

use anyhow::{Context, Result};
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

use qol_conventions::DEFAULT_PORT;

const DEV_GENERATION_DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(8);

pub(crate) fn run() -> Result<()> {
    if qol_process::process_tree_guardian_requested() {
        qol_process::run_process_tree_guardian_entry()
            .context("task command process-tree guardian failed")?;
        return Ok(());
    }
    if let Some(result) = qol_tray::settings_surface::run_from_current_args() {
        return result;
    }
    qol_tray::console_guard::guard_console_pipes();

    if let Some(code) = dispatch_host_cli(host_cli::from_env()) {
        std::process::exit(code);
    }

    #[cfg(feature = "dev")]
    let core_log_controls = qol_tray::logging::init_logger();

    #[cfg(not(feature = "dev"))]
    qol_tray::logging::init_logger();

    qol_tray::logging::log_build_identity();

    qol_tray::lifeline_handoff::adopt_handed_off_fds();

    let generation = qol_tray::dev_generation::current();
    let rolling_restart = qol_tray::dev_generation::is_rolling_restart();
    let mut owns_host_surface = false;
    if generation.is_shadow() {
        log::info!("Starting shadow dev generation");
    } else if rolling_restart {
        log::info!("Starting rolling dev restart");
    } else {
        if let Err(e) = qol_tray::installer::bootstrap_current_install() {
            log::error!("Failed to bootstrap current install: {}", e);
        }

        if is_already_running() {
            eprintln!("qol-tray is already running on port {}", DEFAULT_PORT);
            crate::surfaces::native_notifications::show_already_running();
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

        let drained = qol_tray::config_drain::drain_orphan_runtime_configs();
        if drained > 0 {
            log::info!(
                "[config-drain] folded {drained} orphan plugin config(s) into the host store"
            );
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

        owns_host_surface = true;
        log_binding_restore("startup", hotkeys::restore_desktop_bindings());

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
    }

    log::info!("Starting QoL Tray daemon...");

    #[cfg(feature = "dev")]
    let outcome = tray::platform::run_app(move || app_init(core_log_controls));

    #[cfg(not(feature = "dev"))]
    let outcome = tray::platform::run_app(app_init);

    if owns_host_surface {
        log_binding_restore("shutdown", hotkeys::restore_desktop_bindings());
    }
    outcome
}

fn log_binding_restore(phase: &str, summary: hotkeys::RestoreSummary) {
    for failure in &summary.failures {
        log::warn!("[hotkey-takeover] {phase} restore failed: {failure}");
    }
    if summary.restored == 0
        && summary.abandoned == 0
        && summary.quarantined == 0
        && summary.settled == 0
    {
        return;
    }
    log::info!(
        "[hotkey-takeover] {phase} restore: {} managed shortcut(s) put back, {} left to the user, {} orphan cleanup(s) quarantined, {} settled after desktop restart",
        summary.restored,
        summary.abandoned,
        summary.quarantined,
        summary.settled
    );
}

fn dispatch_host_cli(invocation: host_cli::Invocation) -> Option<i32> {
    match invocation {
        host_cli::Invocation::Daemon => None,
        host_cli::Invocation::Help => {
            print_usage();
            Some(0)
        }
        host_cli::Invocation::Version => {
            println!("qol-tray {}", qol_tray_version());
            Some(0)
        }
        host_cli::Invocation::WriteMode(value) => {
            let exit = write_mode_flag(&value);
            if exit != 0 {
                Some(exit)
            } else {
                None
            }
        }
        host_cli::Invocation::Headless(args) => Some(qol_tray::doctor::run_host_cli(args)),
        host_cli::Invocation::Exec { target, action } => Some(exec_subcommand(&target, &action)),
        host_cli::Invocation::Open(route) => Some(forward_route(&route)),
        host_cli::Invocation::UrlCourier(route) => Some(courier_forward_with_retry(&route)),
        host_cli::Invocation::Url(route) => {
            if is_already_running() {
                Some(forward_route(&route))
            } else {
                let _ = PENDING_COLD_ROUTE.set(route);
                None
            }
        }
        host_cli::Invocation::Invalid => {
            eprintln!("Invalid qol-tray invocation. Run `qol-tray help` for supported forms.");
            Some(2)
        }
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
    println!("    qol-tray doctor                       Run read-only host and plugin checks");
    println!("    qol-tray --write-mode=<dev|prod>      Write mode.json then run the tray");
    println!("    qol-tray --version, -V                Print version and exit");
    println!("    qol-tray help, --help, -h             Print this message and exit");
}

/// Route stashed when a bare `qol://` URL arrives in argv on a Linux cold launch
/// (daemon not yet running); opened once the server is listening. macOS cold
/// launches do not use this - the daemon process never sees the URL in argv;
/// it arrives post-launch via the `openURLs` delegate, which spawns a separate
/// courier process classified by `host_cli`.
static PENDING_COLD_ROUTE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Navigate an already-open UI tab to `route`, falling back to opening a fresh
/// browser tab. Shared by `qol-tray open` and the `qol://` courier.
fn forward_route(route: &str) -> i32 {
    if navigated_open_tab(route) {
        return 0;
    }
    let url = qol_tray::local_http::browser_url(route, DEFAULT_PORT);
    match qol_tray::paths::open_url(&url) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("Failed to open {url}: {e}");
            1
        }
    }
}

/// A macOS courier is spawned by the running daemon's URL delegate, but that
/// daemon's HTTP server may still be binding. Wait briefly (up to ~2s) for it to
/// accept connections so we navigate the live tab instead of opening a dead one,
/// then forward.
fn courier_forward_with_retry(route: &str) -> i32 {
    wait_for_server_ready();
    forward_route(route)
}

fn open_pending_cold_route(route: &str) {
    wait_for_server_ready();
    let url = qol_tray::local_http::browser_url(route, DEFAULT_PORT);
    let _ = qol_tray::paths::open_url(&url);
}

fn exec_subcommand(target: &str, action: &str) -> i32 {
    if target == "shortcut" {
        return exec_shortcut(action);
    }
    if !qol_tray::plugins::manifest::is_valid_plugin_id(target) {
        eprintln!("Invalid plugin id: {target}");
        return 1;
    }
    if !qol_tray::plugins::manifest::is_valid_action_id(action) {
        eprintln!("Invalid action id: {action}");
        return 1;
    }
    fire_action_request(target, action)
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

/// Ask the running daemon to navigate an already-open UI tab to `route`.
/// Returns true only when a tab was subscribed and the event was delivered;
/// any connection error or `delivered:false` returns false so the caller
/// falls back to opening a fresh browser tab.
fn navigated_open_tab(route: &str) -> bool {
    let body = serde_json::json!({ "route": route }).to_string();
    match qol_tray::local_http::post_to_daemon("/api/navigate", &body) {
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
    match qol_tray::local_http::post_to_daemon(&path, "") {
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

fn wait_for_server_ready() -> bool {
    qol_tray::net::wait_for_tcp_ready(
        ([127, 0, 0, 1], DEFAULT_PORT).into(),
        40,
        Duration::from_millis(50),
    )
}

struct InitResult {
    shutdown_tx: broadcast::Sender<()>,
    shutdown_rx: broadcast::Receiver<()>,
    update_available: bool,
    plugin_manager: Arc<Mutex<PluginManager>>,
    feature_registry: Arc<FeatureRegistry>,
    events: Arc<qol_tray::daemon::EventBus>,
    post_pull_task: Option<tokio::task::JoinHandle<()>>,
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
        init.post_pull_task,
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
    let shadow_generation = qol_tray::dev_generation::is_shadow();
    let rolling_restart = qol_tray::dev_generation::is_rolling_restart();
    let update_check = if shadow_generation || rolling_restart {
        None
    } else {
        Some(tokio::spawn(check_for_updates()))
    };
    let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);
    let _state_server = qol_tray::runtime::RuntimeServer::start();
    qol_tray::settings_surface::prewarm();
    qol_tray::doctor::spawn_gpu_driver_sync_watch();
    qol_tray::doctor::spawn_target_cache_watch();
    let plugins_dir = qol_tray::plugins::PluginLoader::ensure_plugin_dir()?;
    let sync_service = Arc::new(qol_tray::sync::SyncService::new(plugins_dir)?);
    if let Ok(config_dir) = qol_tray::paths::shared_config_dir() {
        if let Err(error) = qol_tray::migrations_startup::run_post_auth_if_authed(&config_dir).await
        {
            log::error!("qol-migrations[post-auth] failed: {error:#}");
        }
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
    let (health_tx, health_rx) = qol_tray::plugins::daemon_health::channel();
    #[cfg(not(feature = "dev"))]
    drop(health_rx);
    let ui_port = features::plugin_store::Plugins::start_server(
        plugin_manager.clone(),
        &daemon,
        shutdown_tx.clone(),
        Arc::clone(&sync_service),
        #[cfg(feature = "dev")]
        health_rx,
        #[cfg(feature = "dev")]
        core_log_controls,
    )
    .await?;
    let generation_restart = shadow_generation || rolling_restart;
    let launch_pull_factory = if !shadow_generation && !rolling_restart {
        let sync_for_pull = Arc::clone(&sync_service);
        Some(move || tokio::spawn(async move { sync_for_pull.pull_on_launch().await }))
    } else {
        None
    };
    let (missing_generation_daemons, post_pull_task) = start_local_daemons_before_launch_pull(
        plugin_manager.clone(),
        generation_restart,
        launch_pull_factory,
        shutdown_tx.subscribe(),
    )
    .await?;
    if shadow_generation {
        log::info!("Shadow dev generation: deferring hotkey capture until promotion");
    } else {
        hotkeys::start_capture_with_fallback(plugin_manager.clone());
    }
    qol_tray::plugins::daemon_supervisor::spawn_supervisor(
        plugin_manager.clone(),
        shutdown_tx.subscribe(),
        qol_tray::plugins::daemon_health::HealthPublisher::new(
            health_tx,
            ui_port,
            qol_tray::plugins::daemon_health::default_file_path(),
        ),
    );
    if !missing_generation_daemons.is_empty() {
        let missing = missing_generation_daemons.join(", ");
        if shadow_generation {
            log::warn!("shadow generation daemon(s) not ready before handoff: {missing}");
        } else {
            log::warn!("rolling restart daemon(s) not ready before handoff: {missing}");
        }
    }
    {
        let plugin_manager = plugin_manager.clone();
        tokio::task::spawn_blocking(move || sync_launcher_apps(plugin_manager));
    }
    spawn_config_reconcilers(&daemon.config, &plugin_manager);
    if let Err(error) = qol_tray::dev_generation::write_ready_file(ui_port) {
        log::error!("Failed to write dev generation ready file: {}", error);
    }
    let update_available = finished_update_available(update_check).await;
    Ok(InitResult {
        shutdown_tx,
        shutdown_rx,
        update_available,
        plugin_manager,
        feature_registry,
        events: daemon.events.clone(),
        post_pull_task,
    })
}

async fn finished_update_available(update_check: Option<tokio::task::JoinHandle<bool>>) -> bool {
    match update_check {
        Some(check) if check.is_finished() => check.await.unwrap_or(false),
        Some(_) => {
            log::debug!("Update check continuing after startup");
            false
        }
        None => false,
    }
}

async fn start_local_daemons_before_launch_pull<F>(
    plugin_manager: Arc<Mutex<PluginManager>>,
    generation_restart: bool,
    launch_pull_factory: Option<F>,
    mut shutdown_rx: broadcast::Receiver<()>,
) -> Result<(Vec<String>, Option<tokio::task::JoinHandle<()>>)>
where
    F: FnOnce() -> tokio::task::JoinHandle<anyhow::Result<qol_tray::sync::SyncActionResult>>,
{
    let missing_generation_daemons =
        autostart_plugin_daemons(Arc::clone(&plugin_manager), generation_restart).await;
    let Some(launch_pull_factory) = launch_pull_factory else {
        return Ok((missing_generation_daemons, None));
    };
    let lifecycle_cancellation = plugin_manager
        .lock()
        .map_err(|_| anyhow::anyhow!("plugin manager lock poisoned"))?
        .lifecycle_cancellation();
    if !matches!(
        shutdown_rx.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ) {
        lifecycle_cancellation.cancel();
        return Ok((missing_generation_daemons, None));
    }
    let launch_pull = launch_pull_factory();
    let post_pull_task = tokio::spawn(reconcile_after_launch_pull(
        launch_pull,
        plugin_manager,
        shutdown_rx,
        lifecycle_cancellation,
    ));
    Ok((missing_generation_daemons, Some(post_pull_task)))
}

async fn reconcile_after_launch_pull(
    mut launch_pull: tokio::task::JoinHandle<anyhow::Result<qol_tray::sync::SyncActionResult>>,
    plugin_manager: Arc<Mutex<PluginManager>>,
    mut shutdown_rx: broadcast::Receiver<()>,
    lifecycle_cancellation: Arc<qol_process::CancellationToken>,
) {
    let pull_result = tokio::select! {
        _ = shutdown_rx.recv() => {
            lifecycle_cancellation.cancel();
            launch_pull.abort();
            return;
        }
        result = &mut launch_pull => result,
    };
    let pull_result = match pull_result {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            log::error!("Cloud profile pull on launch failed: {error:#}");
            return;
        }
        Err(error) => {
            if !lifecycle_cancellation.is_cancelled() {
                log::error!("Cloud profile launch task failed: {error}");
            }
            return;
        }
    };
    if !pull_result.applied_remote || lifecycle_cancellation.is_cancelled() {
        return;
    }

    let cancellation_for_reconcile = Arc::clone(&lifecycle_cancellation);
    let reconcile = tokio::task::spawn_blocking(move || {
        if cancellation_for_reconcile.is_cancelled() {
            return;
        }
        materialize_installed_runtime_configs();
        if cancellation_for_reconcile.is_cancelled() {
            return;
        }
        let mut manager = match plugin_manager.lock() {
            Ok(manager) => manager,
            Err(error) => {
                log::error!("Plugin manager mutex poisoned after profile pull: {error}");
                return;
            }
        };
        if let Err(error) = manager.reconcile_profile_generation() {
            log::error!("Failed to reconcile daemons after profile pull: {error:#}");
        }
    });
    tokio::pin!(reconcile);
    tokio::select! {
        _ = shutdown_rx.recv() => {
            lifecycle_cancellation.cancel();
            reconcile.as_ref().abort();
        }
        result = &mut reconcile => {
            if let Err(error) = result {
                log::error!("Post-pull daemon reconciliation task failed: {error}");
            }
        }
    }
}

async fn autostart_plugin_daemons(
    plugin_manager: Arc<Mutex<PluginManager>>,
    generation_restart: bool,
) -> Vec<String> {
    match tokio::task::spawn_blocking(move || {
        if qol_tray::dev_generation::daemon_autostart_held() {
            log::info!("Shadow dev generation: deferring plugin daemon autostart until promotion");
            return Vec::new();
        }
        if let Ok(mut manager) = plugin_manager.lock() {
            manager.reconcile_and_autostart_daemons();
            if generation_restart {
                return manager
                    .wait_for_autostart_daemons_ready(DEV_GENERATION_DAEMON_READY_TIMEOUT);
            }
        } else {
            log::error!("plugin manager lock poisoned during daemon autostart");
        }
        Vec::new()
    })
    .await
    {
        Ok(missing) => missing,
        Err(error) => {
            log::error!("daemon autostart task failed: {}", error);
            Vec::new()
        }
    }
}

fn materialize_installed_runtime_configs() {
    match qol_tray::plugins::PluginConfigManager::new()
        .and_then(|manager| manager.materialize_installed_runtime_configs())
    {
        Ok(count) => log::info!("Materialized runtime config for {count} installed plugin(s)"),
        Err(error) => {
            log::error!("Failed to materialize runtime configs after profile pull: {error:#}")
        }
    }
}

fn sync_launcher_apps(plugin_manager: Arc<Mutex<PluginManager>>) {
    features::launcher_apps::trigger_full_sync_with_manager(&plugin_manager);
}

fn spawn_config_reconcilers(
    config: &qol_tray::daemon::ConfigBus,
    plugin_manager: &Arc<Mutex<PluginManager>>,
) {
    use qol_tray::daemon::ConfigKind;
    use qol_tray::reconcile::spawn_reconciler;

    spawn_reconciler(
        config,
        &[
            ConfigKind::Hotkeys,
            ConfigKind::Plugins,
            ConfigKind::Profile,
        ],
        qol_tray::hotkeys::trigger_reload,
    );

    let pm_for_launcher = plugin_manager.clone();
    spawn_reconciler(
        config,
        &[
            ConfigKind::Shortcuts,
            ConfigKind::Plugins,
            ConfigKind::Profile,
        ],
        move || {
            features::launcher_apps::trigger_full_sync_with_manager(&pm_for_launcher);
        },
    );

    let pm_for_plugins = plugin_manager.clone();
    spawn_reconciler(
        config,
        &[ConfigKind::Plugins, ConfigKind::Profile],
        move || {
            let mut manager = match pm_for_plugins.lock() {
                Ok(manager) => manager,
                Err(error) => {
                    log::error!(
                        "Plugin manager mutex poisoned during generation reconcile: {error}"
                    );
                    return;
                }
            };
            if let Err(error) = manager.reconcile_profile_generation() {
                log::error!("Failed to reconcile plugin generation after config event: {error:#}");
            }
        },
    );
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

    let os_info = qol_tray::paths::current_os_subdir();
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

    crate::surfaces::native_notifications::show_first_run();

    wait_for_server_ready();

    let url = qol_tray::local_http::browser_url("", DEFAULT_PORT);
    let _ = qol_tray::paths::open_url(&url);
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
mod tests {
    use super::{
        finished_update_available, reconcile_after_launch_pull,
        start_local_daemons_before_launch_pull,
    };
    use qol_tray::plugins::PluginManager;
    use std::sync::Arc;
    use tokio::sync::oneshot;
    use tokio::time::{timeout, Duration};

    #[tokio::test(flavor = "current_thread")]
    async fn pending_update_check_does_not_hold_startup() {
        let pending = tokio::spawn(std::future::pending::<bool>());

        let available = timeout(
            Duration::from_millis(50),
            finished_update_available(Some(pending)),
        )
        .await
        .unwrap();

        assert!(!available);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn local_daemons_start_before_a_pending_launch_pull() {
        let manager = Arc::new(std::sync::Mutex::new(PluginManager::new()));
        let (release_tx, release_rx) = oneshot::channel();
        let (missing, post_pull_task) = start_local_daemons_before_launch_pull(
            manager,
            false,
            Some(|| {
                tokio::spawn(async move {
                    release_rx.await.unwrap();
                    anyhow::bail!("test pull remains pending until local startup completes")
                })
            }),
            tokio::sync::broadcast::channel(1).1,
        )
        .await
        .unwrap();
        assert!(missing.is_empty());
        let post_pull_task = post_pull_task.expect("launch pull reconciliation is tracked");
        assert!(!post_pull_task.is_finished());
        release_tx.send(()).unwrap();
        timeout(Duration::from_secs(1), post_pull_task)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_cancels_pending_launch_pull_reconciliation() {
        let manager = Arc::new(std::sync::Mutex::new(PluginManager::new()));
        let cancellation = manager.lock().unwrap().lifecycle_cancellation();
        let (release_tx, release_rx) = oneshot::channel();
        let launch_pull = tokio::spawn(async move {
            release_rx.await.unwrap();
            anyhow::bail!("test pull should have been cancelled")
        });
        let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel(1);
        let task = tokio::spawn(reconcile_after_launch_pull(
            launch_pull,
            Arc::clone(&manager),
            shutdown_rx,
            Arc::clone(&cancellation),
        ));
        tokio::task::yield_now().await;
        shutdown_tx.send(()).unwrap();
        let _ = release_tx.send(());
        timeout(Duration::from_secs(1), task)
            .await
            .unwrap()
            .unwrap();
        assert!(cancellation.is_cancelled());
    }
}
