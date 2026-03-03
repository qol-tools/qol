use super::plugin_ui;
pub(crate) mod assets;
#[cfg(feature = "dev")]
mod dev_handlers;
#[cfg(feature = "dev")]
pub(crate) mod dev_plugin_cpu;
#[cfg(feature = "dev")]
mod dev_runtime;
#[cfg(feature = "dev")]
mod dev_runtime_state;
#[cfg(feature = "dev")]
mod dev_services;
mod helpers;
mod plugin_handlers;
mod plugin_services;
#[cfg(feature = "dev")]
mod restart;
mod settings_handlers;
mod types;

use anyhow::Result;
use axum::http::HeaderValue;
use axum::{
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::sync::{Arc, Mutex};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::daemon::Daemon;
use crate::plugins::{PluginLoader, PluginManager};
use types::*;

pub async fn start_ui_server(
    plugin_manager: Arc<Mutex<PluginManager>>,
    daemon: &Daemon,
) -> Result<u16> {
    let plugins_dir = PluginLoader::default_plugin_dir()?;
    #[cfg(feature = "dev")]
    let plugin_cpu = dev_plugin_cpu::DevPluginCpuService::start(
        plugin_manager.clone(),
        daemon.events.clone(),
    );

    let app_state = AppState {
        plugins_dir: plugins_dir.clone(),
        plugin_manager,
        daemon: daemon.clone(),
        #[cfg(feature = "dev")]
        dev_state: Arc::new(crate::dev::state::DevState::new()),
        #[cfg(feature = "dev")]
        runtime: dev_runtime::new_dev_runtime(),
        #[cfg(feature = "dev")]
        plugin_cpu,
        #[cfg(feature = "dev")]
        restart: restart::default_restart_port(),
    };

    #[cfg(feature = "dev")]
    crate::dev::state::start_discovery(
        &app_state.dev_state,
        &app_state.daemon.events,
        app_state.plugins_dir.clone(),
    );

    let api = Router::new()
        .route("/plugins", get(plugin_handlers::list_plugins))
        .route("/installed", get(plugin_handlers::list_installed))
        .route("/events", get(plugin_handlers::sse_handler))
        .route("/cover/{id}", get(settings_handlers::serve_cover))
        .route("/icon/{bundle_id}", get(settings_handlers::serve_icon))
        .route("/apps", get(settings_handlers::list_apps))
        .route(
            "/plugins/{id}/actions/{action}",
            post(plugin_handlers::execute_plugin_action),
        )
        .route("/install/{id}", post(plugin_handlers::install_plugin))
        .route("/update/{id}", post(plugin_handlers::update_plugin))
        .route("/uninstall/{id}", post(plugin_handlers::uninstall_plugin))
        .route(
            "/plugins/{id}/config",
            get(settings_handlers::get_plugin_config),
        )
        .route(
            "/plugins/{id}/config",
            axum::routing::put(settings_handlers::set_plugin_config),
        )
        .route("/github-token", get(settings_handlers::get_token_status))
        .route("/github-token", post(settings_handlers::set_github_token))
        .route(
            "/github-token",
            axum::routing::delete(settings_handlers::delete_github_token),
        )
        .route("/hotkeys", get(settings_handlers::get_hotkeys))
        .route(
            "/hotkeys",
            axum::routing::put(settings_handlers::set_hotkeys),
        )
        .route("/dev/enabled", get(dev_enabled))
        .route("/version", get(get_version))
        .route("/check-update", get(check_update))
        .route("/self-update", post(self_update));

    #[cfg(feature = "dev")]
    let api = api
        .route("/dev/reload", post(dev_handlers::reload_plugins))
        .route("/dev/recompile-self", post(dev_handlers::recompile_self))
        .route("/dev/links", get(dev_handlers::list_linked_plugins))
        .route("/dev/links", post(dev_handlers::create_link))
        .route(
            "/dev/links/{id}",
            axum::routing::delete(dev_handlers::delete_link),
        )
        .route(
            "/dev/log-controls/{id}",
            axum::routing::put(dev_handlers::upsert_plugin_log_control),
        )
        .route("/dev/log-controls", get(dev_handlers::get_log_controls))
        .route("/dev/discover", post(dev_handlers::trigger_discovery))
        .route(
            "/dev/discovery-state",
            get(dev_handlers::get_discovery_state),
        )
        .route("/dev/build-state", get(dev_handlers::get_build_state))
        .route("/dev/plugin-cpu", get(dev_handlers::get_plugin_cpu))
        .route(
            "/dev/mock-check-update",
            get(dev_handlers::mock_check_update),
        )
        .route("/dev/mock-targets", get(dev_handlers::list_mock_targets))
        .route(
            "/dev/mock-targets/start",
            post(dev_handlers::start_mock_targets),
        )
        .route(
            "/dev/mock-targets/stop",
            post(dev_handlers::stop_mock_targets),
        )
        .route(
            "/dev/mock-plugin-build",
            post(dev_handlers::mock_plugin_build),
        )
        .route(
            "/dev/mock-plugin-build/stop",
            post(dev_handlers::stop_mock_plugin_build),
        )
        .route(
            "/dev/mock-self-recompile",
            post(dev_handlers::mock_self_recompile),
        )
        .route(
            "/dev/mock-self-recompile/stop",
            post(dev_handlers::stop_mock_self_recompile),
        )
        .route(
            "/dev/mock-self-update",
            post(dev_handlers::mock_self_update),
        )
        .route(
            "/dev/mock-self-update/stop",
            post(dev_handlers::stop_mock_self_update),
        );

    let api = api.with_state(app_state);

    let no_cache = SetResponseHeaderLayer::overriding(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );

    let task_runner = super::super::task_runner::router();

    let app = Router::new()
        .nest("/api", api)
        .nest("/api/task-runner", task_runner)
        .nest("/plugins", plugin_ui::router(plugins_dir))
        .route("/", get(assets::serve_embedded_index))
        .route("/{*path}", get(assets::serve_embedded))
        .layer(no_cache);

    let (listener, port) = bind_listener().await?;

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            log::error!("UI server error: {}", e);
        }
    });

    Ok(port)
}

async fn bind_listener() -> Result<(tokio::net::TcpListener, u16)> {
    let address = format!("127.0.0.1:{}", DEFAULT_UI_SERVER_PORT);
    let listener = tokio::net::TcpListener::bind(&address).await?;
    Ok((listener, DEFAULT_UI_SERVER_PORT))
}

async fn dev_enabled() -> Json<bool> {
    Json(cfg!(feature = "dev"))
}

async fn get_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

async fn check_update() -> Json<serde_json::Value> {
    let available = crate::updates::check_for_updates().await.unwrap_or(false);
    let latest = crate::updates::latest_version().map(String::from);
    Json(serde_json::json!({ "available": available, "latest": latest }))
}

async fn self_update(State(state): State<AppState>) -> impl IntoResponse {
    let events = state.daemon.events.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::updates::download_and_install(events.clone()).await {
            log::error!("Self-update failed: {}", e);
            events.send(crate::daemon::DaemonEvent::UpdateFailed {
                message: e.to_string(),
            });
        }
    });
    StatusCode::ACCEPTED
}
