use super::plugin_ui;
mod assets;
#[cfg(feature = "dev")]
mod dev_handlers;
#[cfg(feature = "dev")]
mod dev_runtime;
mod helpers;
mod settings_handlers;
mod types;

use crate::paths::is_safe_path_component;
use anyhow::Result;
use axum::http::HeaderValue;
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::sync::{Arc, Mutex};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::daemon::Daemon;
use crate::plugins::{PluginLoader, PluginManager};
use helpers::{
    extract_actions, infer_load_error, is_newer_version, read_installed_plugin_dirs,
    read_manifest_without_validation, read_plugin_version, reload_manager_and_notify,
};
use types::*;

pub async fn start_ui_server(
    plugin_manager: Arc<Mutex<PluginManager>>,
    daemon: &Daemon,
) -> Result<u16> {
    let plugins_dir = PluginLoader::default_plugin_dir()?;

    let app_state = AppState {
        plugins_dir: plugins_dir.clone(),
        plugin_manager,
        daemon: daemon.clone(),
    };

    let api = Router::new()
        .route("/plugins", get(list_plugins))
        .route("/installed", get(list_installed))
        .route("/events", get(sse_handler))
        .route("/cover/{id}", get(settings_handlers::serve_cover))
        .route(
            "/plugins/{id}/actions/{action}",
            post(execute_plugin_action),
        )
        .route("/install/{id}", post(install_plugin))
        .route("/update/{id}", post(update_plugin))
        .route("/uninstall/{id}", post(uninstall_plugin))
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
        .route("/dev/discover", post(dev_handlers::trigger_discovery))
        .route(
            "/dev/discovery-state",
            get(dev_handlers::get_discovery_state),
        )
        .route("/dev/build-state", get(dev_handlers::get_build_state))
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

async fn list_plugins(
    axum::extract::Query(query): axum::extract::Query<PluginsQuery>,
) -> Result<Json<PluginsResponse>, (StatusCode, String)> {
    use super::github::{cache_age_secs, GitHubClient};

    log::info!("API /plugins called (refresh={})", query.refresh);

    let client = GitHubClient::new("qol-tools");
    let plugins_dir = match PluginLoader::default_plugin_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("Failed to determine config directory: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to determine plugin directory".to_string(),
            ));
        }
    };

    let installed_versions: std::collections::HashMap<String, String> =
        read_installed_plugin_dirs(&plugins_dir)
            .into_iter()
            .filter_map(|(id, path)| {
                let version = read_plugin_version(&path).ok()?;
                Some((id, version))
            })
            .collect();

    let cache_age = cache_age_secs();

    let metadata_list = client
        .list_plugins_cached(query.refresh)
        .await
        .map_err(|e| {
            log::error!("Failed to fetch plugins: {}", e);
            (
                StatusCode::SERVICE_UNAVAILABLE,
                format!("Failed to fetch plugins: {:#}", e),
            )
        })?;
    log::info!("Got {} plugins", metadata_list.len());
    let plugins = metadata_list
        .into_iter()
        .filter(|m| m.supports_current_platform())
        .map(|m| {
            let installed_version = installed_versions.get(&m.id).cloned();
            PluginInfo {
                id: m.id.clone(),
                name: m.name,
                description: m.description,
                version: m.version,
                installed: installed_version.is_some(),
                installed_version,
            }
        })
        .collect();

    Ok(Json(PluginsResponse {
        plugins,
        cache_age_secs: cache_age,
    }))
}

async fn install_plugin(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<PluginInfo>, (StatusCode, String)> {
    use super::installer::PluginInstaller;

    if !is_safe_path_component(&id) {
        return Err((StatusCode::BAD_REQUEST, "Invalid plugin ID".to_string()));
    }

    log::info!("Install requested for plugin: {}", id);

    if let Err(e) = std::fs::create_dir_all(&state.plugins_dir) {
        log::error!("Failed to get plugins directory: {}", e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to access plugins directory".to_string(),
        ));
    }

    let plugins_dir = state.plugins_dir.clone();

    let installer = PluginInstaller::new(plugins_dir.clone());
    let repo_url = format!("https://github.com/qol-tools/{}.git", id);

    installer.install(&repo_url, &id).await.map_err(|e| {
        log::error!("Failed to install plugin {}: {}", id, e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Installation failed: {:#}", e),
        )
    })?;

    reload_manager_and_notify(&state);

    log::info!("Plugin {} installed successfully", id);
    let version = read_plugin_version(&plugins_dir.join(&id)).unwrap_or_else(|_| "unknown".into());
    Ok(Json(PluginInfo {
        id: id.clone(),
        name: id.clone(),
        description: "Installed successfully".to_string(),
        installed_version: Some(version.clone()),
        version,
        installed: true,
    }))
}

async fn update_plugin(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Json<UninstallResult> {
    use super::installer::PluginInstaller;

    if !is_safe_path_component(&id) {
        return Json(UninstallResult {
            success: false,
            message: "Invalid plugin ID".to_string(),
        });
    }

    log::info!("Update requested for plugin: {}", id);

    let installer = PluginInstaller::new(state.plugins_dir.clone());
    let repo_url = format!("https://github.com/qol-tools/{}.git", id);

    if let Err(e) = installer.update(&repo_url, &id).await {
        log::error!("Failed to update plugin {}: {}", id, e);
        return Json(UninstallResult {
            success: false,
            message: format!("Update failed: {:#}", e),
        });
    }

    if let Ok(version) = read_plugin_version(&state.plugins_dir.join(&id)) {
        super::github::update_cached_version(&id, &version);
    }

    reload_manager_and_notify(&state);

    log::info!("Plugin {} updated successfully", id);
    Json(UninstallResult {
        success: true,
        message: "Updated successfully".to_string(),
    })
}

async fn sse_handler(State(state): State<AppState>) -> impl IntoResponse {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use tokio_stream::wrappers::BroadcastStream;
    use tokio_stream::StreamExt;

    let rx = state.daemon.events.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| {
        result.ok().and_then(|event| {
            serde_json::to_string(&event)
                .ok()
                .map(|json| Ok::<_, std::convert::Infallible>(Event::default().data(json)))
        })
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn uninstall_plugin(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Json<UninstallResult> {
    use super::installer::PluginInstaller;

    if !is_safe_path_component(&id) {
        return Json(UninstallResult {
            success: false,
            message: "Invalid plugin ID".to_string(),
        });
    }

    log::info!("Uninstall requested for plugin: {}", id);

    let installer = PluginInstaller::new(state.plugins_dir.clone());

    if let Err(e) = installer.uninstall(&id).await {
        log::error!("Failed to uninstall plugin {}: {}", id, e);
        return Json(UninstallResult {
            success: false,
            message: "Uninstall failed".to_string(),
        });
    }

    reload_manager_and_notify(&state);

    log::info!("Plugin {} uninstalled successfully", id);
    Json(UninstallResult {
        success: true,
        message: "Uninstalled successfully".to_string(),
    })
}

async fn execute_plugin_action(
    Path((id, action)): Path<(String, String)>,
    State(state): State<AppState>,
) -> (StatusCode, Json<ExecuteActionResult>) {
    if !is_safe_path_component(&id) {
        return (
            StatusCode::BAD_REQUEST,
            Json(ExecuteActionResult {
                success: false,
                message: "Invalid plugin ID".to_string(),
            }),
        );
    }

    match crate::plugins::action_executor::try_execute_action(&state.plugin_manager, &id, &action) {
        Ok(()) => (
            StatusCode::OK,
            Json(ExecuteActionResult {
                success: true,
                message: "Action dispatched".to_string(),
            }),
        ),
        Err(error @ crate::plugins::action_executor::ActionExecutionError::PluginNotFound(_)) => {
            log::warn!("Plugin action rejected for {}::{}: {}", id, action, error);
            (
                StatusCode::NOT_FOUND,
                Json(ExecuteActionResult {
                    success: false,
                    message: error.to_string(),
                }),
            )
        }
        Err(error @ crate::plugins::action_executor::ActionExecutionError::InvalidActionId(_)) => {
            log::warn!("Plugin action rejected for {}::{}: {}", id, action, error);
            (
                StatusCode::BAD_REQUEST,
                Json(ExecuteActionResult {
                    success: false,
                    message: error.to_string(),
                }),
            )
        }
        Err(
            error @ crate::plugins::action_executor::ActionExecutionError::MissingActionMapping {
                ..
            },
        ) => {
            log::warn!("Plugin action rejected for {}::{}: {}", id, action, error);
            (
                StatusCode::BAD_REQUEST,
                Json(ExecuteActionResult {
                    success: false,
                    message: error.to_string(),
                }),
            )
        }
        Err(error) => {
            log::error!("Plugin action failed for {}::{}: {}", id, action, error);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ExecuteActionResult {
                    success: false,
                    message: "Action execution failed".to_string(),
                }),
            )
        }
    }
}

async fn list_installed(
    State(state): State<AppState>,
) -> Result<Json<InstalledPluginsResponse>, StatusCode> {
    use super::github::read_cache;
    use std::collections::HashMap;

    let revision = state.daemon.events.plugins_revision();

    let manager = state.plugin_manager.lock().map_err(|e| {
        log::error!("Plugin manager mutex poisoned: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let cached_versions: HashMap<String, String> = read_cache()
        .map(|c| c.plugins.into_iter().map(|p| (p.id, p.version)).collect())
        .unwrap_or_default();

    let mut plugins_by_id: HashMap<String, InstalledPlugin> = manager
        .plugins()
        .map(|plugin| {
            let cover_path = plugin.path.join("cover.png");
            let ui_path = plugin.path.join("ui").join("index.html");
            let available_version = cached_versions.get(&plugin.id).cloned();
            let update_available = available_version
                .as_ref()
                .map(|av| is_newer_version(av, &plugin.manifest.plugin.version))
                .unwrap_or(false);

            let actions = extract_actions(&plugin.manifest.menu.items);

            (
                plugin.id.clone(),
                InstalledPlugin {
                    id: plugin.id.clone(),
                    name: plugin.manifest.plugin.name.clone(),
                    description: plugin.manifest.plugin.description.clone(),
                    version: plugin.manifest.plugin.version.clone(),
                    loaded: true,
                    load_error: None,
                    has_cover: cover_path.exists(),
                    has_ui: ui_path.exists(),
                    available_version,
                    update_available,
                    actions,
                },
            )
        })
        .collect();

    drop(manager);

    for (id, plugin_dir) in read_installed_plugin_dirs(&state.plugins_dir) {
        if plugins_by_id.contains_key(&id) {
            continue;
        }

        let manifest = read_manifest_without_validation(&plugin_dir);
        let (name, description, version, actions) = match manifest.as_ref() {
            Some(m) => (
                m.plugin.name.clone(),
                m.plugin.description.clone(),
                m.plugin.version.clone(),
                extract_actions(&m.menu.items),
            ),
            None => (
                id.clone(),
                "Plugin manifest could not be parsed".to_string(),
                "unknown".to_string(),
                Vec::new(),
            ),
        };

        let available_version = cached_versions.get(&id).cloned();
        let update_available = available_version
            .as_ref()
            .map(|av| version != "unknown" && is_newer_version(av, &version))
            .unwrap_or(false);

        let load_error = infer_load_error(&id, &plugin_dir, manifest.as_ref());

        plugins_by_id.insert(
            id.clone(),
            InstalledPlugin {
                id,
                name,
                description,
                version,
                loaded: false,
                load_error,
                has_cover: plugin_dir.join("cover.png").exists(),
                has_ui: plugin_dir.join("ui").join("index.html").exists(),
                available_version,
                update_available,
                actions,
            },
        );
    }

    let plugins: Vec<InstalledPlugin> = plugins_by_id.into_values().collect();

    Ok(Json(InstalledPluginsResponse { revision, plugins }))
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
