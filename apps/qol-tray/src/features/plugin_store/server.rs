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
use crate::daemon::{Daemon, DaemonEvent};
#[cfg(feature = "dev")]
use crate::daemon::DiscoveryStatus;
use crate::hotkeys::trigger_reload;
#[cfg(feature = "dev")]
use crate::dev;

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
struct PluginAction {
    id: String,
    label: String,
}

#[derive(Serialize)]
struct InstalledPlugin {
    id: String,
    name: String,
    description: String,
    version: String,
    has_cover: bool,
    has_ui: bool,
    available_version: Option<String>,
    update_available: bool,
    actions: Vec<PluginAction>,
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
) -> Result<()> {
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
        .route("/version", get(get_version));

    #[cfg(feature = "dev")]
    let api = api
        .route("/dev/reload", post(reload_plugins))
        .route("/dev/links", get(list_linked_plugins))
        .route("/dev/links", post(create_link))
        .route("/dev/links/{id}", axum::routing::delete(delete_link))
        .route("/dev/discover", post(trigger_discovery))
        .route("/dev/discovery-state", get(get_discovery_state));

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

    let listener = tokio::net::TcpListener::bind("127.0.0.1:42700").await?;

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            log::error!("UI server error: {}", e);
        }
    });

    Ok(())
}

fn get_installed_plugin_ids(plugins_dir: &std::path::Path) -> std::collections::HashSet<String> {
    if !plugins_dir.exists() {
        return std::collections::HashSet::new();
    }

    std::fs::read_dir(plugins_dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default()
}

async fn list_plugins(
    axum::extract::Query(query): axum::extract::Query<PluginsQuery>,
) -> Json<PluginsResponse> {
    use super::github::{GitHubClient, cache_age_secs};

    log::info!("API /plugins called (refresh={})", query.refresh);

    let client = GitHubClient::new("qol-tools");
    let plugins_dir = match PluginLoader::default_plugin_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("Failed to determine config directory: {}", e);
            return Json(PluginsResponse { plugins: vec![], cache_age_secs: None });
        }
    };

    let installed_plugins = get_installed_plugin_ids(&plugins_dir);

    let cache_age = cache_age_secs();

    let plugins = match client.list_plugins_cached(query.refresh).await {
        Ok(metadata_list) => {
            log::info!("Got {} plugins", metadata_list.len());
            metadata_list
                .into_iter()
                .filter(|m| m.supports_current_platform())
                .map(|m| PluginInfo {
                    id: m.id.clone(),
                    name: m.name,
                    description: m.description,
                    version: m.version,
                    installed: installed_plugins.contains(&m.id),
                })
                .collect()
        }
        Err(e) => {
            log::error!("Failed to fetch plugins: {}", e);
            vec![]
        }
    };
    
    Json(PluginsResponse {
        plugins,
        cache_age_secs: cache_age,
    })
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

    let plugins_dir = PluginLoader::ensure_plugin_dir().map_err(|e| {
        log::error!("Failed to get plugins directory: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, "Failed to access plugins directory".to_string())
    })?;

    let installer = PluginInstaller::new(plugins_dir.clone());
    let repo_url = format!("https://github.com/qol-tools/{}.git", id);

    installer.install(&repo_url, &id).await.map_err(|e| {
        log::error!("Failed to install plugin {}: {}", id, e);
        (StatusCode::INTERNAL_SERVER_ERROR, "Installation failed".to_string())
    })?;

    reload_manager_and_notify(&state);

    log::info!("Plugin {} installed successfully", id);
    let version = read_plugin_version(&plugins_dir.join(&id)).unwrap_or_else(|_| "unknown".into());
    Ok(Json(PluginInfo {
        id: id.clone(),
        name: id.clone(),
        description: "Installed successfully".to_string(),
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

    if let Err(e) = installer.update(&id).await {
        log::error!("Failed to update plugin {}: {}", id, e);
        return Json(UninstallResult {
            success: false,
            message: "Update failed".to_string(),
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
    state.daemon.events.send(DaemonEvent::PluginsChanged);
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

async fn list_installed(
    State(state): State<AppState>,
) -> Result<Json<Vec<InstalledPlugin>>, StatusCode> {
    use super::github::read_cache;
    use std::collections::HashMap;

    let manager = state.plugin_manager.lock().map_err(|e| {
        log::error!("Plugin manager mutex poisoned: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let cached_versions: HashMap<String, String> = read_cache()
        .map(|c| c.plugins.into_iter().map(|p| (p.id, p.version)).collect())
        .unwrap_or_default();

    let plugins: Vec<InstalledPlugin> = manager.plugins()
        .map(|plugin| {
            let cover_path = plugin.path.join("cover.png");
            let ui_path = plugin.path.join("ui").join("index.html");
            let available_version = cached_versions.get(&plugin.id).cloned();
            let update_available = available_version
                .as_ref()
                .map(|av| is_newer_version(av, &plugin.manifest.plugin.version))
                .unwrap_or(false);

            let actions = extract_actions(&plugin.manifest.menu.items);

            InstalledPlugin {
                id: plugin.id.clone(),
                name: plugin.manifest.plugin.name.clone(),
                description: plugin.manifest.plugin.description.clone(),
                version: plugin.manifest.plugin.version.clone(),
                has_cover: cover_path.exists(),
                has_ui: ui_path.exists(),
                available_version,
                update_available,
                actions,
            }
        })
        .collect();

    Ok(Json(plugins))
}

async fn dev_enabled() -> Json<bool> {
    Json(cfg!(feature = "dev"))
}

async fn get_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(feature = "dev")]
async fn reload_plugins(State(state): State<AppState>) -> impl IntoResponse {
    log::info!("Developer reload requested");
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

fn extract_actions(items: &[crate::plugins::MenuItem]) -> Vec<PluginAction> {
    use crate::plugins::MenuItem;

    items.iter().flat_map(|item| match item {
        MenuItem::Action { id, label, .. } => {
            vec![PluginAction { id: id.clone(), label: label.clone() }]
        }
        MenuItem::Submenu { items, .. } => extract_actions(items),
        MenuItem::Checkbox { .. } | MenuItem::Separator => vec![],
    }).collect()
}

fn is_newer_version(available: &str, installed: &str) -> bool {
    use crate::version::Version;
    Version::parse(available).is_newer_than(&Version::parse(installed))
}

const MAX_COVER_SIZE: usize = 5 * 1024 * 1024;

async fn serve_cover(
    Path(plugin_id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if !is_safe_path_component(&plugin_id) {
        return (StatusCode::BAD_REQUEST, "Invalid plugin ID").into_response();
    }

    let cover_path = state.plugins_dir.join(&plugin_id).join("cover.png");

    if !cover_path.exists() {
        return (StatusCode::NOT_FOUND, "Cover not found").into_response();
    }

    let data = match tokio::fs::read(&cover_path).await {
        Ok(data) => data,
        Err(e) => {
            log::error!("Failed to read cover image: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read cover").into_response();
        }
    };

    if data.len() > MAX_COVER_SIZE {
        return (StatusCode::PAYLOAD_TOO_LARGE, "Cover image too large").into_response();
    }

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
async fn list_linked_plugins(
    State(state): State<AppState>,
) -> Result<Json<Vec<dev::LinkedPlugin>>, StatusCode> {
    dev::list_linked_plugins(&state.plugins_dir)
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
    let source = std::path::Path::new(&req.path);

    match dev::create_link(source, &state.plugins_dir) {
        Ok(_) => {
            state.daemon.start_discovery(state.plugins_dir.clone());
            (StatusCode::OK, "Link created").into_response()
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

    match dev::remove_link(&id, &state.plugins_dir) {
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
    let guard = state.daemon.state.discovery.read().unwrap();
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
