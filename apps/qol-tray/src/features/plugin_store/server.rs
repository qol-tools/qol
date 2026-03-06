use super::plugin_ui;
pub(crate) mod assets;
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

pub async fn start_ui_server(
    plugin_manager: Arc<Mutex<PluginManager>>,
    daemon: &Daemon,
) -> Result<u16> {
    let (app_state, plugins_dir) = AppState::new(plugin_manager, daemon)?;
    #[cfg(feature = "dev")]
    start_dev_discovery(&app_state);
    let app = assemble_app(app_state, plugins_dir);
    let (listener, port) = bind_listener().await?;
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            log::error!("UI server error: {}", e);
        }
    });
    Ok(port)
}

fn api_router(app_state: AppState) -> Router {
    let api = plugin_handlers::routes()
        .merge(settings::routes())
        .merge(meta_handlers::routes());
    #[cfg(feature = "dev")]
    let api = api
        .merge(dev_handlers::routes())
        .merge(dev_link_handlers::routes())
        .merge(dev_state_handlers::routes())
        .merge(dev_mock_handlers::routes());
    api.with_state(app_state)
        .layer(middleware::from_fn(security::reject_cross_site_mutations))
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
