use super::plugin_ui;
pub(crate) mod assets;
mod boot;
#[cfg(feature = "dev")]
mod dev_core_log_handlers;
#[cfg(feature = "dev")]
mod dev_gate;
#[cfg(feature = "dev")]
mod dev_handlers;
#[cfg(feature = "dev")]
mod dev_health_handlers;
#[cfg(feature = "dev")]
mod dev_link_handlers;
#[cfg(feature = "dev")]
mod dev_mock_handlers;
#[cfg(feature = "dev")]
pub(crate) mod dev_plugin_cpu;
#[cfg(feature = "dev")]
mod dev_runtime;
#[cfg(feature = "dev")]
mod dev_runtime_state;
#[cfg(feature = "dev")]
mod dev_services;
#[cfg(feature = "dev")]
mod dev_state_handlers;
#[cfg(feature = "dev")]
mod dev_validation;
mod helpers;
mod logs_handlers;
mod meta_handlers;
mod plugin_handlers;
mod plugin_services;
#[cfg(feature = "dev")]
mod restart;
pub(crate) mod security;
mod settings;
mod types;
mod ui_trace_handlers;

use anyhow::Result;
use axum::{
    http::{header, HeaderValue},
    middleware,
    routing::get,
    Router,
};
use std::path::PathBuf;
#[cfg(feature = "dev")]
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
#[cfg(feature = "dev")]
use std::time::Duration;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::daemon::Daemon;
use crate::plugins::PluginManager;
use types::*;

#[cfg(feature = "dev")]
const PROMOTED_DAEMON_READY_TIMEOUT: Duration = Duration::from_secs(8);

pub(crate) async fn start_ui_server(
    plugin_manager: Arc<Mutex<PluginManager>>,
    daemon: &Daemon,
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
    sync_service: Arc<crate::features::profile::sync::SyncService>,
    #[cfg(feature = "dev")] daemon_health: tokio::sync::watch::Receiver<
        crate::plugins::daemon_health::HealthSnapshot,
    >,
    #[cfg(feature = "dev")] core_log_controls: crate::logging::CoreControlsHandle,
) -> Result<u16> {
    let github_auth_service = Arc::new(crate::features::github_auth::GitHubAuthService::new());
    let (app_state, plugins_dir) = AppState::new(
        plugin_manager,
        daemon,
        shutdown_tx,
        github_auth_service,
        sync_service,
        #[cfg(feature = "dev")]
        daemon_health,
        #[cfg(feature = "dev")]
        core_log_controls,
    )?;
    if !crate::dev_generation::is_shadow() {
        start_sync_loop(&app_state);
        #[cfg(feature = "dev")]
        start_dev_discovery(&app_state);
        #[cfg(feature = "dev")]
        schedule_post_restart_rebuild(&app_state);
    } else {
        log::info!("Shadow dev generation: skipping sync loop and dev discovery");
    }
    let (listener, port) = bind_listener().await?;
    let http_security = security::HttpSecurity::initialize(port)?;
    let app = assemble_app(app_state, plugins_dir, http_security);
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            log::error!("UI server error: {}", e);
        }
    });
    Ok(port)
}

#[cfg(feature = "dev")]
async fn promote_shadow_to_stable(app_state: AppState) -> Result<u16> {
    if !claim_promotion(
        crate::dev_generation::is_shadow(),
        &app_state.promoted_to_stable,
    )? {
        return Ok(qol_conventions::DEFAULT_PORT);
    }
    let listener = match bind_listener_at(qol_conventions::DEFAULT_PORT).await {
        Ok((listener, port)) => (listener, port),
        Err(error) => {
            app_state.promoted_to_stable.store(false, Ordering::Release);
            return Err(error);
        }
    };
    let (listener, port) = listener;
    let http_security = security::HttpSecurity::initialize(port)?;
    let app = assemble_app(
        app_state.clone(),
        app_state.plugins_dir.clone(),
        http_security,
    );
    if let Err(error) = crate::dev_generation::drain_predecessor_daemons_for_promotion() {
        app_state.promoted_to_stable.store(false, Ordering::Release);
        return Err(error);
    }
    if !super::platform::bind_public_runtime_socket() {
        app_state.promoted_to_stable.store(false, Ordering::Release);
        anyhow::bail!("failed to bind promoted runtime state socket");
    }
    super::ACTIVE_SERVER_PORT.store(port, Ordering::Relaxed);
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, app).await {
            log::error!("Promoted UI server error: {}", error);
        }
    });
    crate::dev_generation::promote_to_stable();
    crate::settings_surface::prewarm();
    complete_promotion_in_background(app_state);
    Ok(port)
}

#[cfg(feature = "dev")]
fn claim_promotion(is_shadow: bool, promoted: &std::sync::atomic::AtomicBool) -> Result<bool> {
    if promoted.load(Ordering::Acquire) {
        return Ok(false);
    }
    if !is_shadow {
        anyhow::bail!("only a shadow dev generation can be promoted");
    }
    Ok(promoted
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok())
}

#[cfg(feature = "dev")]
fn complete_promotion_in_background(app_state: AppState) {
    tokio::spawn(async move {
        let repair_state = app_state.clone();
        let dev_links_repaired = tokio::task::spawn_blocking(repair_startup_state_after_promotion)
            .await
            .unwrap_or_else(|error| {
                log::error!("promoted startup repair task failed: {}", error);
                false
            });
        if dev_links_repaired {
            tokio::task::spawn_blocking(move || {
                helpers::reload_manager_and_notify_without_profile_sync(&repair_state);
            })
            .await
            .unwrap_or_else(|error| {
                log::error!("promoted plugin reload task failed: {}", error);
            });
        }
        let plugin_manager = app_state.plugin_manager.clone();
        let missing_daemons = tokio::task::spawn_blocking(move || match plugin_manager.lock() {
            Ok(mut manager) => {
                manager.autostart_daemons();
                manager.wait_for_autostart_daemons_ready(PROMOTED_DAEMON_READY_TIMEOUT)
            }
            Err(_) => {
                log::error!("plugin manager lock poisoned during promoted daemon autostart");
                Vec::new()
            }
        })
        .await
        .unwrap_or_else(|error| {
            log::error!("promoted daemon autostart task failed: {}", error);
            Vec::new()
        });
        if missing_daemons.is_empty() {
            log::info!("Promoted dev generation: daemon autostart ready");
        } else {
            log::warn!(
                "Promoted dev generation: daemon(s) not ready before handoff: {}",
                missing_daemons.join(", ")
            );
        }
        crate::hotkeys::start_capture_with_fallback(app_state.plugin_manager.clone());
        start_sync_loop(&app_state);
        start_dev_discovery(&app_state);
    });
}

#[cfg(feature = "dev")]
fn repair_startup_state_after_promotion() -> bool {
    repair_boot_selection_after_promotion();
    let report = crate::doctor::auto_fix_startup();
    log::info!(
        "Promoted dev generation: startup repairs attempted={} applied={} failures={}",
        report.attempted,
        report.applied,
        report.failures.len()
    );
    let was_broken = report.before.outcomes().any(|outcome| {
        outcome.id == "dev_link_paths"
            && !matches!(outcome.status, crate::doctor::OutcomeStatus::Ok)
    });
    let is_healthy = report.after.outcomes().any(|outcome| {
        outcome.id == "dev_link_paths" && matches!(outcome.status, crate::doctor::OutcomeStatus::Ok)
    });
    was_broken && is_healthy
}

#[cfg(feature = "dev")]
fn repair_boot_selection_after_promotion() {
    let Ok(config_dir) = crate::paths::shared_config_dir() else {
        return;
    };
    let env = crate::installer::boot_environment::default_boot_environment();
    let lister = crate::dev::boot_contract::GitWorktreeLister;
    let probe = crate::dev::boot_contract::FsBinaryProbe;
    match crate::dev::boot_contract::repair_autostart_for_selection(
        env.as_ref(),
        &config_dir,
        &lister,
        &probe,
    ) {
        Ok(report) if report.wrote_autostart => log::info!(
            "[boot-contract] promoted generation re-aligned autostart to {}",
            report.target.binary().display()
        ),
        Ok(_) => {}
        Err(error) => log::error!("[boot-contract] promoted autostart repair failed: {error:#}"),
    }
}

#[cfg(feature = "dev")]
fn schedule_post_restart_rebuild(app_state: &AppState) {
    let branch = match std::env::var("QOL_DEV_WORKTREE_BRANCH") {
        Ok(b) if !b.is_empty() => b,
        _ => return,
    };
    std::env::remove_var("QOL_DEV_WORKTREE_BRANCH");
    log::info!(
        "[worktree] post-restart plugin rebuild for branch: {}",
        branch
    );
    let state = app_state.clone();
    tokio::task::spawn_blocking(move || {
        if let Err(e) = dev_services::queue_reload(&state, Some(branch)) {
            log::warn!("[worktree] post-restart rebuild skipped: {}", e);
        }
    });
}

fn api_router(app_state: AppState, http_security: security::HttpSecurity) -> Router {
    let api = plugin_handlers::routes()
        .merge(settings::routes())
        .merge(crate::features::github_auth::routes())
        .merge(crate::features::auth::routes())
        .merge(meta_handlers::routes())
        .merge(ui_trace_handlers::routes())
        .merge(logs_handlers::routes());
    #[cfg(feature = "dev")]
    let api = api.merge(dev_api_router());
    api.with_state(app_state)
        .layer(middleware::from_fn_with_state(
            http_security,
            security::require_api_access,
        ))
}

#[cfg(feature = "dev")]
fn dev_api_router() -> Router<AppState> {
    Router::new()
        .merge(dev_handlers::routes())
        .merge(dev_health_handlers::routes())
        .merge(dev_link_handlers::routes())
        .merge(dev_state_handlers::routes())
        .merge(dev_mock_handlers::routes())
        .merge(dev_core_log_handlers::routes())
        .route_layer(middleware::from_fn(dev_gate::require_dev_mode))
}

fn assemble_app(
    app_state: AppState,
    plugins_dir: PathBuf,
    http_security: security::HttpSecurity,
) -> Router {
    let api = api_router(app_state, http_security.clone());
    let no_cache = SetResponseHeaderLayer::overriding(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );
    let task_runner = super::super::task_runner::router().layer(middleware::from_fn_with_state(
        http_security.clone(),
        security::require_api_access,
    ));
    Router::new()
        .nest("/api", api)
        .nest("/api/task-runner", task_runner)
        .nest("/plugins", plugin_ui::router(plugins_dir))
        .route("/", get(assets::serve_embedded_index))
        .route("/{*path}", get(assets::serve_embedded))
        .layer(no_cache)
        .layer(middleware::from_fn_with_state(
            http_security,
            security::require_local_host,
        ))
}

fn start_sync_loop(app_state: &AppState) {
    let state = app_state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(
            crate::features::profile::sync::SyncService::auto_push_interval(),
        );
        loop {
            interval.tick().await;
            let result = match state.sync_service.auto_push_if_dirty().await {
                Ok(result) => result,
                Err(error) => {
                    log::error!("Cloud sync auto-push failed: {error:#}");
                    continue;
                }
            };
            if result.applied_remote {
                helpers::reload_manager_and_notify_without_profile_sync(&state);
            }
        }
    });
}

#[cfg(feature = "dev")]
fn start_dev_discovery(app_state: &AppState) {
    crate::dev::state::start_discovery(
        &app_state.dev_state,
        &app_state.daemon.events,
        app_state.plugins_dir.clone(),
    );
}

async fn bind_listener() -> Result<(tokio::net::TcpListener, u16)> {
    bind_listener_at(crate::dev_generation::current().ui_bind_port()).await
}

async fn bind_listener_at(port: u16) -> Result<(tokio::net::TcpListener, u16)> {
    let address = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&address).await?;
    let port = listener.local_addr()?.port();
    Ok((listener, port))
}

#[cfg(all(test, feature = "dev"))]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn promotion_claim_is_idempotent_after_the_role_changes() {
        let promoted = AtomicBool::new(false);

        assert!(claim_promotion(true, &promoted).unwrap());
        assert!(!claim_promotion(false, &promoted).unwrap());
        assert!(!claim_promotion(true, &promoted).unwrap());
    }

    #[test]
    fn stable_generation_cannot_claim_a_new_promotion() {
        let promoted = AtomicBool::new(false);

        assert!(claim_promotion(false, &promoted).is_err());
        assert!(!promoted.load(Ordering::Acquire));
    }
}
