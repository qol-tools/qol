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
mod security;
mod settings;
mod types;

use anyhow::Result;
use axum::{
    http::{header, HeaderValue},
    middleware,
    routing::get,
    Router,
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::daemon::Daemon;
use crate::plugins::PluginManager;
use types::*;

pub(crate) async fn start_ui_server(
    plugin_manager: Arc<Mutex<PluginManager>>,
    daemon: &Daemon,
    sync_service: Arc<crate::features::profile::sync::SyncService>,
    #[cfg(feature = "dev")] core_log_controls: crate::logging::CoreControlsHandle,
) -> Result<u16> {
    let github_auth_service = Arc::new(crate::features::github_auth::GitHubAuthService::new());
    let (app_state, plugins_dir) = AppState::new(
        plugin_manager,
        daemon,
        github_auth_service,
        sync_service,
        #[cfg(feature = "dev")]
        core_log_controls,
    )?;
    start_sync_loop(&app_state);
    #[cfg(feature = "dev")]
    start_dev_discovery(&app_state);
    #[cfg(feature = "dev")]
    schedule_post_restart_rebuild(&app_state);
    let app = assemble_app(app_state, plugins_dir);
    let (listener, port) = bind_listener().await?;
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            log::error!("UI server error: {}", e);
        }
    });
    Ok(port)
}

#[cfg(feature = "dev")]
fn schedule_post_restart_rebuild(app_state: &AppState) {
    let branch = match std::env::var("QOL_DEV_WORKTREE_BRANCH") {
        Ok(b) if !b.is_empty() => b,
        _ => return,
    };
    // Consume the env var so it doesn't persist across future restarts
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

fn api_router(app_state: AppState) -> Router {
    let api = plugin_handlers::routes()
        .merge(settings::routes())
        .merge(crate::features::github_auth::routes())
        .merge(crate::features::auth::routes())
        .merge(meta_handlers::routes())
        .merge(logs_handlers::routes());
    #[cfg(feature = "dev")]
    let api = api.merge(dev_api_router());
    api.with_state(app_state)
        .layer(middleware::from_fn(security::reject_cross_site_mutations))
}

#[cfg(feature = "dev")]
fn dev_api_router() -> Router<AppState> {
    Router::new()
        .merge(dev_handlers::routes())
        .merge(dev_link_handlers::routes())
        .merge(dev_state_handlers::routes())
        .merge(dev_mock_handlers::routes())
        .merge(dev_core_log_handlers::routes())
        .route_layer(middleware::from_fn(dev_gate::require_dev_mode))
}

fn assemble_app(app_state: AppState, plugins_dir: PathBuf) -> Router {
    let api = api_router(app_state);
    let no_cache = SetResponseHeaderLayer::overriding(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );
    let task_runner = super::super::task_runner::router()
        .layer(middleware::from_fn(security::reject_cross_site_mutations));
    Router::new()
        .nest("/api", api)
        .nest("/api/task-runner", task_runner)
        .nest("/plugins", plugin_ui::router(plugins_dir))
        .route("/", get(assets::serve_embedded_index))
        .route("/{*path}", get(assets::serve_embedded))
        .layer(no_cache)
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
    let address = format!("127.0.0.1:{}", DEFAULT_UI_SERVER_PORT);
    let listener = tokio::net::TcpListener::bind(&address).await?;
    Ok((listener, DEFAULT_UI_SERVER_PORT))
}
