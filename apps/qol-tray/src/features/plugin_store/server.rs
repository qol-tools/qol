use super::plugin_ui;

use crate::paths::is_safe_path_component;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use axum::{
    extract::{Path, State},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
    http::{StatusCode, header},
};
use serde::{Deserialize, Serialize};
use tower_http::set_header::SetResponseHeaderLayer;
use axum::http::HeaderValue;
use anyhow::Result;
use rust_embed::Embed;

use crate::plugins::{PluginConfigManager, PluginLoader, PluginManager};
use crate::daemon::Daemon;
#[cfg(feature = "dev")]
use crate::daemon::DaemonEvent;
#[cfg(feature = "dev")]
use crate::daemon::{BuildResultInfo, DiscoveryStatus};
use crate::hotkeys::trigger_reload;
#[cfg(feature = "dev")]
use crate::dev;

const DEFAULT_UI_SERVER_PORT: u16 = 42700;

#[derive(Clone)]
struct AppState {
    plugins_dir: PathBuf,
    plugin_manager: Arc<Mutex<PluginManager>>,
    daemon: Daemon,
}

#[derive(Embed)]
#[folder = "ui/"]
struct UiAssets;

#[derive(Serialize)]
struct PluginInfo {
    id: String,
    name: String,
    description: String,
    version: String,
    installed: bool,
    installed_version: Option<String>,
}

#[derive(Serialize)]
struct PluginsResponse {
    plugins: Vec<PluginInfo>,
    cache_age_secs: Option<u64>,
}

#[derive(Deserialize, Default)]
struct PluginsQuery {
    #[serde(default)]
    refresh: bool,
}

#[derive(Serialize)]
struct UninstallResult {
    success: bool,
    message: String,
}

#[derive(Serialize)]
struct ExecuteActionResult {
    success: bool,
    message: String,
}

#[derive(Serialize)]
struct PluginAction {
    id: String,
    label: String,
    kind: crate::plugins::ActionType,
}

#[derive(Serialize)]
struct InstalledPlugin {
    id: String,
    name: String,
    description: String,
    version: String,
    loaded: bool,
    load_error: Option<String>,
    has_cover: bool,
    has_ui: bool,
    available_version: Option<String>,
    update_available: bool,
    actions: Vec<PluginAction>,
}

#[derive(Serialize)]
struct InstalledPluginsResponse {
    revision: u64,
    plugins: Vec<InstalledPlugin>,
}

#[derive(Deserialize)]
struct TokenRequest {
    token: String,
}

#[derive(Serialize)]
struct TokenStatus {
    has_token: bool,
}

async fn serve_embedded(Path(path): Path<String>) -> impl IntoResponse {
    serve_embedded_file(&path)
}

async fn serve_embedded_index() -> impl IntoResponse {
    serve_embedded_file("index.html")
}

fn serve_embedded_file(path: &str) -> impl IntoResponse {
    let mime = if path.ends_with(".html") {
        "text/html"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".js") {
        "application/javascript"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    };

    match UiAssets::get(path) {
        Some(content) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, mime)],
            content.data.into_owned(),
        ).into_response(),
        None => (StatusCode::NOT_FOUND, "Not found").into_response(),
    }
}

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
        .route("/cover/{id}", get(serve_cover))
        .route("/plugins/{id}/actions/{action}", post(execute_plugin_action))
        .route("/install/{id}", post(install_plugin))
        .route("/update/{id}", post(update_plugin))
        .route("/uninstall/{id}", post(uninstall_plugin))
        .route("/plugins/{id}/config", get(get_plugin_config))
        .route("/plugins/{id}/config", axum::routing::put(set_plugin_config))
        .route("/github-token", get(get_token_status))
        .route("/github-token", post(set_github_token))
        .route("/github-token", axum::routing::delete(delete_github_token))
        .route("/hotkeys", get(get_hotkeys))
        .route("/hotkeys", axum::routing::put(set_hotkeys))
        .route("/dev/enabled", get(dev_enabled))
        .route("/version", get(get_version))
        .route("/check-update", get(check_update))
        .route("/self-update", post(self_update));

    #[cfg(feature = "dev")]
    let api = api
        .route("/dev/reload", post(reload_plugins))
        .route("/dev/recompile-self", post(recompile_self))
        .route("/dev/links", get(list_linked_plugins))
        .route("/dev/links", post(create_link))
        .route("/dev/links/{id}", axum::routing::delete(delete_link))
        .route("/dev/discover", post(trigger_discovery))
        .route("/dev/discovery-state", get(get_discovery_state))
        .route("/dev/mock-check-update", get(mock_check_update))
        .route("/dev/mock-self-update", post(mock_self_update));

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
        .route("/", get(serve_embedded_index))
        .route("/{*path}", get(serve_embedded))
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


fn read_installed_plugin_dirs(plugins_dir: &std::path::Path) -> Vec<(String, std::path::PathBuf)> {
    if !plugins_dir.exists() {
        return Vec::new();
    }

    std::fs::read_dir(plugins_dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).ok()?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return None;
            }
            let id = entry.file_name().into_string().ok()?;
            if id.starts_with('.') || id.ends_with(".backup") || !is_safe_path_component(&id) {
                return None;
            }
            Some((id, path))
        })
        .collect()
}

async fn list_plugins(
    axum::extract::Query(query): axum::extract::Query<PluginsQuery>,
) -> Result<Json<PluginsResponse>, (StatusCode, String)> {
    use super::github::{GitHubClient, cache_age_secs};

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

    let metadata_list = client.list_plugins_cached(query.refresh).await.map_err(|e| {
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
        (StatusCode::INTERNAL_SERVER_ERROR, format!("Installation failed: {:#}", e))
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

fn read_plugin_version(plugin_dir: &std::path::Path) -> Result<String, ()> {
    let manifest_path = plugin_dir.join("plugin.toml");
    let content = std::fs::read_to_string(&manifest_path).map_err(|_| ())?;
    let manifest: crate::plugins::PluginManifest = toml::from_str(&content).map_err(|_| ())?;
    Ok(manifest.plugin.version)
}

fn reload_manager_and_notify(state: &AppState) {
    let mut manager = match state.plugin_manager.lock() {
        Ok(m) => m,
        Err(e) => {
            log::error!("Plugin manager mutex poisoned: {}", e);
            return;
        }
    };
    if let Err(e) = manager.reload_plugins() {
        log::error!("Failed to reload plugins: {}", e);
    }
    trigger_reload();
    state.daemon.events.send_plugins_changed();
}

async fn sse_handler(
    State(state): State<AppState>,
) -> impl IntoResponse {
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

    let mut plugins_by_id: HashMap<String, InstalledPlugin> = manager.plugins()
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

#[cfg(feature = "dev")]
async fn reload_plugins(State(state): State<AppState>) -> impl IntoResponse {
    log::info!("Developer reload requested");

    state.daemon.events.send(DaemonEvent::BuildStarted);

    let dev_links = config_dir_then(|d| dev::load_dev_links(d));
    let build_results = dev::build_linked_plugins(&dev_links);
    let results: Vec<BuildResultInfo> = build_results
        .into_iter()
        .map(|r| BuildResultInfo {
            plugin_id: r.plugin_id,
            success: r.success,
            output: r.output,
        })
        .collect();

    let all_succeeded = results.is_empty() || results.iter().all(|r| r.success);
    state.daemon.events.send(DaemonEvent::BuildComplete { results });

    if !all_succeeded {
        return (StatusCode::OK, "Build completed with errors").into_response();
    }

    let mut manager = match state.plugin_manager.lock() {
        Ok(m) => m,
        Err(e) => {
            log::error!("Plugin manager mutex poisoned: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Plugin manager lock failed").into_response();
        }
    };
    match manager.reload_plugins() {
        Ok(_) => {
            log::info!("Plugins reloaded successfully");
            (StatusCode::OK, "Plugins reloaded").into_response()
        }
        Err(e) => {
            log::error!("Failed to reload plugins: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed: {}", e)).into_response()
        }
    }
}

#[cfg(feature = "dev")]
async fn recompile_self(State(state): State<AppState>) -> impl IntoResponse {
    log::info!("Developer self recompile requested");

    let events = state.daemon.events.clone();
    tokio::spawn(async move {
        let progress_events = events.clone();
        let result = tokio::task::spawn_blocking(move || {
            dev::build_qol_tray_self_with_progress(|percent, phase| {
                progress_events.send(DaemonEvent::SelfRecompileProgress { percent, phase });
            })
        })
        .await;

        match result {
            Ok(build) if build.success => {
                events.send(DaemonEvent::SelfRecompileComplete);
            }
            Ok(build) => {
                let message = build_failure_message(&build.output);
                log::error!("Self recompile failed: {}", message);
                events.send(DaemonEvent::SelfRecompileFailed { message });
            }
            Err(e) => {
                let message = format!("Self recompile worker failed: {}", e);
                log::error!("{}", message);
                events.send(DaemonEvent::SelfRecompileFailed { message });
            }
        }
    });

    StatusCode::ACCEPTED
}

#[cfg(feature = "dev")]
fn build_failure_message(output: &str) -> String {
    output
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "Self recompile failed".to_string())
}

fn extract_actions(items: &[crate::plugins::MenuItem]) -> Vec<PluginAction> {
    use crate::plugins::MenuItem;

    let mut actions = Vec::new();
    let mut collect = |item: &MenuItem| match item {
        MenuItem::Action {
            id, label, action, ..
        } => actions.push(PluginAction {
                id: id.clone(),
                label: label.clone(),
                kind: *action,
            }),
        MenuItem::Checkbox { .. } | MenuItem::Separator | MenuItem::Submenu { .. } => {}
    };
    crate::plugins::manifest::walk_menu_items(items, &mut collect);
    actions
}

fn is_newer_version(available: &str, installed: &str) -> bool {
    use crate::version::Version;
    Version::parse(available).is_newer_than(&Version::parse(installed))
}

fn read_manifest_without_validation(
    plugin_dir: &std::path::Path,
) -> Option<crate::plugins::PluginManifest> {
    let manifest_path = plugin_dir.join("plugin.toml");
    let content = std::fs::read_to_string(manifest_path).ok()?;
    toml::from_str(&content).ok()
}

fn infer_load_error(
    plugin_id: &str,
    plugin_dir: &std::path::Path,
    manifest: Option<&crate::plugins::PluginManifest>,
) -> Option<String> {
    let Some(manifest) = manifest else {
        return Some("Invalid plugin.toml".to_string());
    };

    if !manifest.plugin.supports_current_platform() {
        return Some(format!(
            "Unsupported platform: current platform is {}",
            std::env::consts::OS
        ));
    }

    crate::plugins::PluginLoader::load_plugin_with_id(plugin_id, plugin_dir)
        .err()
        .map(|e| format!("{:#}", e))
}

const MAX_COVER_SIZE: usize = 5 * 1024 * 1024;

async fn serve_cover(
    Path(plugin_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if !is_safe_path_component(&plugin_id) {
        return (StatusCode::BAD_REQUEST, "Invalid plugin ID").into_response();
    }

    let plugin_root = state.plugins_dir.join(&plugin_id);
    let plugin_meta = match tokio::fs::symlink_metadata(&plugin_root).await {
        Ok(meta) => meta,
        Err(_) => return (StatusCode::NOT_FOUND, "Cover not found").into_response(),
    };
    if plugin_meta.file_type().is_symlink() {
        return (StatusCode::FORBIDDEN, "Invalid cover path").into_response();
    }

    let cover_path = plugin_root.join("cover.png");
    let cover_meta = match tokio::fs::symlink_metadata(&cover_path).await {
        Ok(meta) => meta,
        Err(_) => return (StatusCode::NOT_FOUND, "Cover not found").into_response(),
    };
    if cover_meta.file_type().is_symlink() {
        return (StatusCode::FORBIDDEN, "Invalid cover path").into_response();
    }
    if !cover_meta.is_file() {
        return (StatusCode::NOT_FOUND, "Cover not found").into_response();
    }

    let canonical_root = match tokio::fs::canonicalize(&plugin_root).await {
        Ok(path) => path,
        Err(_) => return (StatusCode::NOT_FOUND, "Cover not found").into_response(),
    };
    let canonical_cover = match tokio::fs::canonicalize(&cover_path).await {
        Ok(path) => path,
        Err(_) => return (StatusCode::NOT_FOUND, "Cover not found").into_response(),
    };
    if !canonical_cover.starts_with(&canonical_root) {
        return (StatusCode::FORBIDDEN, "Invalid cover path").into_response();
    }

    let cover_size = match tokio::fs::metadata(&canonical_cover).await {
        Ok(meta) => meta.len() as usize,
        Err(e) => {
            log::error!("Failed to read cover metadata: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read cover").into_response();
        }
    };

    if cover_size > MAX_COVER_SIZE {
        return (StatusCode::PAYLOAD_TOO_LARGE, "Cover image too large").into_response();
    }

    let data = match tokio::fs::read(&canonical_cover).await {
        Ok(data) => data,
        Err(e) => {
            log::error!("Failed to read cover image: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read cover").into_response();
        }
    };

    (StatusCode::OK, [(header::CONTENT_TYPE, "image/png")], data).into_response()
}

const MAX_CONFIG_SIZE: usize = 1024 * 1024;

async fn get_plugin_config(Path(plugin_id): Path<String>) -> impl IntoResponse {
    if !is_safe_path_component(&plugin_id) {
        return (StatusCode::BAD_REQUEST, "Invalid plugin ID").into_response();
    }

    let config = match PluginConfigManager::new().and_then(|m| m.get_config(&plugin_id)) {
        Ok(Some(config)) => config,
        Ok(None) => return (StatusCode::NOT_FOUND, "Config not found").into_response(),
        Err(e) => {
            log::error!("Failed to read config: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read config").into_response();
        }
    };

    match serde_json::to_vec(&config) {
        Ok(data) => (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], data).into_response(),
        Err(e) => {
            log::error!("Failed to serialize config: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to serialize config").into_response()
        }
    }
}

async fn set_plugin_config(
    Path(plugin_id): Path<String>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    if !is_safe_path_component(&plugin_id) {
        return (StatusCode::BAD_REQUEST, "Invalid plugin ID").into_response();
    }

    if body.len() > MAX_CONFIG_SIZE {
        return (StatusCode::PAYLOAD_TOO_LARGE, "Config too large").into_response();
    }

    let config: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Invalid JSON in config: {}", e);
            return (StatusCode::BAD_REQUEST, "Invalid JSON").into_response();
        }
    };

    match PluginConfigManager::new().and_then(|m| m.set_config(&plugin_id, config)) {
        Ok(()) => {
            log::info!("Config saved for plugin: {}", plugin_id);
            (StatusCode::OK, "Config saved").into_response()
        }
        Err(e) => {
            log::error!("Failed to save config: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save config").into_response()
        }
    }
}

async fn get_token_status() -> Json<TokenStatus> {
    Json(TokenStatus {
        has_token: super::github::get_stored_token().is_some(),
    })
}

async fn set_github_token(Json(payload): Json<TokenRequest>) -> impl IntoResponse {
    use super::github::TokenValidationError;

    if let Err(e) = super::github::validate_token(&payload.token).await {
        let (status, label) = match &e {
            TokenValidationError::Empty | TokenValidationError::Invalid(_) => {
                (StatusCode::BAD_REQUEST, "Rejected")
            }
            TokenValidationError::Upstream(_) => {
                (StatusCode::BAD_GATEWAY, "Upstream failure")
            }
        };
        log::warn!("{} GitHub token: {}", label, e);
        return (status, e.to_string()).into_response();
    }

    if let Err(e) = super::github::store_token(&payload.token) {
        log::error!("Failed to store GitHub token: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to store token".to_string()).into_response();
    }

    log::info!("GitHub token stored successfully");
    (StatusCode::OK, "Token stored".to_string()).into_response()
}

async fn delete_github_token() -> impl IntoResponse {
    if let Err(e) = super::github::delete_token() {
        log::error!("Failed to delete GitHub token: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to delete token".to_string()).into_response();
    }

    log::info!("GitHub token deleted");
    (StatusCode::OK, "Token deleted".to_string()).into_response()
}

async fn get_hotkeys() -> impl IntoResponse {
    use crate::hotkeys::HotkeyManager;

    let manager = match HotkeyManager::new() {
        Ok(m) => m,
        Err(e) => {
            log::error!("Failed to create HotkeyManager: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to load hotkeys").into_response();
        }
    };

    let config = match manager.load_config() {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to load hotkey config: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to load hotkeys").into_response();
        }
    };

    let json = match serde_json::to_vec(&config) {
        Ok(j) => j,
        Err(e) => {
            log::error!("Failed to serialize hotkey config: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to serialize hotkeys").into_response();
        }
    };

    (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], json).into_response()
}

async fn set_hotkeys(body: axum::body::Bytes) -> impl IntoResponse {
    use crate::hotkeys::{HotkeyConfig, HotkeyManager};

    if body.len() > MAX_CONFIG_SIZE {
        return (StatusCode::PAYLOAD_TOO_LARGE, "Config too large").into_response();
    }

    let config: HotkeyConfig = match serde_json::from_slice(&body) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Invalid hotkey config JSON: {}", e);
            return (StatusCode::BAD_REQUEST, "Invalid JSON").into_response();
        }
    };

    let manager = match HotkeyManager::new() {
        Ok(m) => m,
        Err(e) => {
            log::error!("Failed to create HotkeyManager: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save hotkeys").into_response();
        }
    };

    if let Err(e) = manager.save_config(&config) {
        log::error!("Failed to save hotkey config: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to save hotkeys").into_response();
    }

    trigger_reload();
    log::info!("Hotkey config saved");
    (StatusCode::OK, "Hotkeys saved").into_response()
}

#[cfg(feature = "dev")]
fn config_dir_then<T>(f: impl FnOnce(&std::path::Path) -> T) -> T
where
    T: Default,
{
    match crate::paths::config_dir() {
        Ok(dir) => f(&dir),
        Err(e) => {
            log::error!("Failed to determine config directory: {}", e);
            T::default()
        }
    }
}

#[cfg(feature = "dev")]
async fn list_linked_plugins(
    State(_state): State<AppState>,
) -> Result<Json<Vec<dev::LinkedPlugin>>, StatusCode> {
    let config_dir = crate::paths::config_dir().map_err(|e| {
        log::error!("Failed to determine config directory: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    dev::list_linked_plugins(&config_dir)
        .map(Json)
        .map_err(|e| {
            log::error!("Failed to list linked plugins: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
}

#[cfg(feature = "dev")]
async fn create_link(
    State(state): State<AppState>,
    Json(req): Json<dev::LinkRequest>,
) -> impl IntoResponse {
    let config_dir = match crate::paths::config_dir() {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to determine config directory: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Config dir unavailable".to_string()).into_response();
        }
    };
    let source = std::path::Path::new(&req.path);

    match dev::create_link(source, &config_dir) {
        Ok(_) => {
            state.daemon.start_discovery(state.plugins_dir.clone());
            (StatusCode::OK, "Link created".to_string()).into_response()
        }
        Err(e) if e.contains("Already linked") => (StatusCode::CONFLICT, e).into_response(),
        Err(e) if e.contains("does not exist") || e.contains("No plugin.toml") => {
            (StatusCode::BAD_REQUEST, e).into_response()
        }
        Err(e) => {
            log::error!("Failed to create link: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e).into_response()
        }
    }
}

#[cfg(feature = "dev")]
async fn delete_link(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if !is_safe_path_component(&id) {
        return (StatusCode::BAD_REQUEST, "Invalid plugin ID".to_string()).into_response();
    }

    let config_dir = match crate::paths::config_dir() {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to determine config directory: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Config dir unavailable".to_string()).into_response();
        }
    };

    match dev::remove_link(&id, &config_dir) {
        Ok(()) => {
            state.daemon.start_discovery(state.plugins_dir.clone());
            (StatusCode::OK, "Unlinked".to_string()).into_response()
        }
        Err(e) => {
            log::error!("Failed to remove link for {}: {}", id, e);
            (StatusCode::BAD_REQUEST, e).into_response()
        }
    }
}

#[cfg(feature = "dev")]
#[derive(Serialize)]
struct DiscoveryStateResponse {
    status: String,
    plugins: Vec<crate::daemon::DiscoveredPluginInfo>,
}

#[cfg(feature = "dev")]
async fn get_discovery_state(
    State(state): State<AppState>,
) -> Json<DiscoveryStateResponse> {
    let guard = match state.daemon.state.discovery.read() {
        Ok(guard) => guard,
        Err(error) => {
            log::error!("Discovery state lock poisoned: {}", error);
            return Json(DiscoveryStateResponse {
                status: "idle".to_string(),
                plugins: Vec::new(),
            });
        }
    };
    let status = match guard.status {
        DiscoveryStatus::Idle => "idle",
        DiscoveryStatus::Discovering => "discovering",
        DiscoveryStatus::Complete => "complete",
    };
    Json(DiscoveryStateResponse {
        status: status.to_string(),
        plugins: guard.plugins.clone(),
    })
}

#[cfg(feature = "dev")]
async fn trigger_discovery(State(state): State<AppState>) -> impl IntoResponse {
    log::info!("Discovery refresh requested");
    state.daemon.start_discovery(state.plugins_dir.clone());
    StatusCode::OK
}

#[cfg(feature = "dev")]
async fn mock_check_update() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "available": true, "latest": "99.0.0" }))
}

#[cfg(feature = "dev")]
async fn mock_self_update(State(state): State<AppState>) -> impl IntoResponse {
    let events = state.daemon.events.clone();
    tokio::spawn(async move {
        for i in 0..=100 {
            events.send(DaemonEvent::UpdateProgress { percent: i });
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
        events.send(DaemonEvent::UpdateComplete);
    });
    StatusCode::ACCEPTED
}
