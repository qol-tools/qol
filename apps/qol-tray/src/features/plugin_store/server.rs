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

use crate::plugins::{PluginLoader, PluginManager};
use crate::hotkeys::trigger_reload;

#[derive(Clone)]
struct AppState {
    plugins_dir: PathBuf,
    plugin_manager: Arc<Mutex<PluginManager>>,
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


async fn serve_embedded(axum::extract::Path(path): axum::extract::Path<String>) -> impl IntoResponse {
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

pub async fn start_ui_server(plugin_manager: Arc<Mutex<PluginManager>>) -> Result<()> {
    let plugins_dir = PluginLoader::default_plugin_dir()?;

    let app_state = AppState {
        plugins_dir: plugins_dir.clone(),
        plugin_manager,
    };

    let api = Router::new()
        .route("/plugins", get(list_plugins))
        .route("/installed", get(list_installed))
        .route("/cover/:id", get(serve_cover))
        .route("/install/:id", post(install_plugin))
        .route("/update/:id", post(update_plugin))
        .route("/uninstall/:id", post(uninstall_plugin))
        .route("/plugins/:id/config", get(get_plugin_config))
        .route("/plugins/:id/config", axum::routing::put(set_plugin_config))
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
        .route("/dev/links/:id", axum::routing::delete(delete_link))
        .route("/dev/discover", get(discover_plugins));

    let api = api.with_state(app_state);

    let no_cache = SetResponseHeaderLayer::overriding(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );

    let app = Router::new()
        .nest("/api", api)
        .nest("/plugins", plugin_ui::router(plugins_dir))
        .route("/", get(serve_embedded_index))
        .route("/*path", get(serve_embedded))
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

async fn install_plugin(Path(id): Path<String>) -> Result<Json<PluginInfo>, (StatusCode, String)> {
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

async fn update_plugin(Path(id): Path<String>) -> Json<UninstallResult> {
    use super::installer::PluginInstaller;

    if !is_safe_path_component(&id) {
        return Json(UninstallResult {
            success: false,
            message: "Invalid plugin ID".to_string(),
        });
    }

    log::info!("Update requested for plugin: {}", id);

    let plugins_dir = match PluginLoader::default_plugin_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("Failed to get plugins directory: {}", e);
            return Json(UninstallResult {
                success: false,
                message: "Failed to access plugins directory".to_string(),
            });
        }
    };

    let installer = PluginInstaller::new(plugins_dir.clone());

    if let Err(e) = installer.update(&id).await {
        log::error!("Failed to update plugin {}: {}", id, e);
        return Json(UninstallResult {
            success: false,
            message: "Update failed".to_string(),
        });
    }

    if let Ok(version) = read_plugin_version(&plugins_dir.join(&id)) {
        super::github::update_cached_version(&id, &version);
    }

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

async fn uninstall_plugin(Path(id): Path<String>) -> Json<UninstallResult> {
    use super::installer::PluginInstaller;

    if !is_safe_path_component(&id) {
        return Json(UninstallResult {
            success: false,
            message: "Invalid plugin ID".to_string(),
        });
    }

    log::info!("Uninstall requested for plugin: {}", id);

    let plugins_dir = match PluginLoader::default_plugin_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("Failed to get plugins directory: {}", e);
            return Json(UninstallResult {
                success: false,
                message: "Failed to access plugins directory".to_string(),
            });
        }
    };

    let installer = PluginInstaller::new(plugins_dir);

    if let Err(e) = installer.uninstall(&id).await {
        log::error!("Failed to uninstall plugin {}: {}", id, e);
        return Json(UninstallResult {
            success: false,
            message: "Uninstall failed".to_string(),
        });
    }

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

async fn get_plugin_config(Path(plugin_id): Path<String>) -> impl IntoResponse {
    if !is_safe_path_component(&plugin_id) {
        return (StatusCode::BAD_REQUEST, "Invalid plugin ID").into_response();
    }

    let manager = match crate::plugins::PluginConfigManager::new() {
        Ok(m) => m,
        Err(e) => {
            log::error!("Failed to create config manager: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to access config").into_response();
        }
    };

    let config = match manager.get_config(&plugin_id) {
        Ok(Some(config)) => config,
        Ok(None) => {
            return (StatusCode::NOT_FOUND, "Config not found").into_response();
        }
        Err(e) => {
            log::error!("Failed to read config: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read config").into_response();
        }
    };

    let data = match serde_json::to_vec(&config) {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to serialize config: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to serialize config").into_response();
        }
    };

    (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], data).into_response()
}

async fn set_plugin_config(
    Path(plugin_id): Path<String>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    if !is_safe_path_component(&plugin_id) {
        return (StatusCode::BAD_REQUEST, "Invalid plugin ID").into_response();
    }

    let config = match serde_json::from_slice::<serde_json::Value>(&body) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Invalid JSON in config: {}", e);
            return (StatusCode::BAD_REQUEST, "Invalid JSON").into_response();
        }
    };

    let manager = match crate::plugins::PluginConfigManager::new() {
        Ok(m) => m,
        Err(e) => {
            log::error!("Failed to create config manager: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to access config").into_response();
        }
    };

    if let Err(e) = manager.set_config(&plugin_id, config) {
        log::error!("Failed to write config: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to write config").into_response();
    }

    log::info!("Config saved for plugin: {}", plugin_id);
    (StatusCode::OK, "Config saved").into_response()
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
#[derive(Serialize)]
struct LinkedPlugin {
    id: String,
    name: String,
    is_symlink: bool,
    target: Option<String>,
}

#[cfg(feature = "dev")]
#[derive(Serialize)]
struct DiscoveredPlugin {
    id: String,
    name: String,
    path: String,
    already_linked: bool,
    installed_not_linked: bool,
}

#[cfg(feature = "dev")]
#[derive(Deserialize)]
struct CreateLinkRequest {
    path: String,
}

#[cfg(feature = "dev")]
async fn list_linked_plugins(
    State(state): State<AppState>,
) -> Result<Json<Vec<LinkedPlugin>>, StatusCode> {
    let plugins_dir = &state.plugins_dir;

    if !plugins_dir.exists() {
        return Ok(Json(vec![]));
    }

    let entries = std::fs::read_dir(plugins_dir).map_err(|e| {
        log::error!("Failed to read plugins dir: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let mut plugins = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "backup") {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();

        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let is_symlink = metadata.file_type().is_symlink();
        let target = if is_symlink {
            std::fs::read_link(&path).ok().map(|p| p.to_string_lossy().to_string())
        } else {
            None
        };

        let name = std::fs::read_to_string(path.join("plugin.toml"))
            .ok()
            .and_then(|s| toml::from_str::<crate::plugins::PluginManifest>(&s).ok())
            .map(|m| m.plugin.name)
            .unwrap_or_else(|| id.clone());

        plugins.push(LinkedPlugin { id, name, is_symlink, target });
    }

    plugins.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(plugins))
}

#[cfg(feature = "dev")]
async fn create_link(
    State(state): State<AppState>,
    Json(req): Json<CreateLinkRequest>,
) -> impl IntoResponse {
    use std::path::Path;

    let source = Path::new(&req.path);
    if !source.exists() {
        return (StatusCode::BAD_REQUEST, "Source path does not exist").into_response();
    }

    if !source.join("plugin.toml").exists() {
        return (StatusCode::BAD_REQUEST, "No plugin.toml found in source").into_response();
    }

    let plugin_id = match source.file_name() {
        Some(name) => name.to_string_lossy().to_string(),
        None => return (StatusCode::BAD_REQUEST, "Invalid path").into_response(),
    };

    let link_path = state.plugins_dir.join(&plugin_id);

    if let Err(e) = backup_existing_if_not_symlink(&link_path) {
        return (StatusCode::CONFLICT, e).into_response();
    }

    if let Err(e) = create_symlink(source, &link_path) {
        log::error!("Failed to create symlink: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to create link").into_response();
    }

    log::info!("Created plugin link: {} -> {}", plugin_id, req.path);
    (StatusCode::OK, "Link created").into_response()
}

#[cfg(feature = "dev")]
fn backup_existing_if_not_symlink(path: &std::path::Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| format!("Failed to check path: {}", e))?;

    if metadata.file_type().is_symlink() {
        return Err("Already linked".to_string());
    }

    let backup_path = path.with_extension("backup");
    if backup_path.exists() {
        std::fs::remove_dir_all(&backup_path)
            .map_err(|e| format!("Failed to remove old backup: {}", e))?;
    }

    std::fs::rename(path, &backup_path)
        .map_err(|e| format!("Failed to backup existing: {}", e))
}

#[cfg(feature = "dev")]
fn create_symlink(source: &std::path::Path, link: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(source, link)
    }
}

#[cfg(feature = "dev")]
fn remove_symlink(path: &std::path::Path) -> Result<(), String> {
    if !path.exists() {
        return Err("Plugin not found".to_string());
    }

    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| format!("Failed to check link: {}", e))?;

    if !metadata.file_type().is_symlink() {
        return Err("Not a symlink - use uninstall instead".to_string());
    }

    std::fs::remove_file(path)
        .map_err(|e| format!("Failed to remove link: {}", e))
}

#[cfg(feature = "dev")]
async fn delete_link(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if !is_safe_path_component(&id) {
        return (StatusCode::BAD_REQUEST, "Invalid plugin ID".to_string()).into_response();
    }

    let link_path = state.plugins_dir.join(&id);

    if let Err(e) = remove_symlink(&link_path) {
        log::error!("Failed to remove symlink for {}: {}", id, e);
        return (StatusCode::BAD_REQUEST, e).into_response();
    }

    if let Err(e) = restore_from_backup(&link_path) {
        log::warn!("No backup to restore for {}: {}", id, e);
    }

    log::info!("Unlinked plugin: {}", id);
    (StatusCode::OK, "Unlinked").into_response()
}

#[cfg(feature = "dev")]
fn restore_from_backup(path: &std::path::Path) -> Result<(), String> {
    let backup_path = path.with_extension("backup");
    if !backup_path.exists() {
        return Err("No backup exists".to_string());
    }

    std::fs::rename(&backup_path, path)
        .map_err(|e| format!("Failed to restore backup: {}", e))
}

#[cfg(feature = "dev")]
async fn discover_plugins(
    State(state): State<AppState>,
) -> Json<Vec<DiscoveredPlugin>> {
    let plugins_dir = &state.plugins_dir;

    let mut discovered: Vec<DiscoveredPlugin> = get_plugin_search_dirs()
        .into_iter()
        .filter(|d| d.exists())
        .flat_map(scan_dir_for_plugins)
        .map(|mut p| {
            let (linked, installed) = check_install_status(plugins_dir, &p.id, &p.path);
            p.already_linked = linked;
            p.installed_not_linked = installed;
            p
        })
        .filter(|p| !p.already_linked)
        .collect();

    discovered.sort_by(|a, b| a.name.cmp(&b.name));
    Json(discovered)
}

#[cfg(feature = "dev")]
fn check_install_status(plugins_dir: &std::path::Path, id: &str, target: &str) -> (bool, bool) {
    let link_path = plugins_dir.join(id);
    if !link_path.exists() {
        return (false, false);
    }

    let Ok(meta) = std::fs::symlink_metadata(&link_path) else {
        return (false, true);
    };

    if !meta.file_type().is_symlink() {
        return (false, true);
    }

    let Ok(resolved) = std::fs::read_link(&link_path) else {
        return (false, true);
    };

    let is_linked_to_target = resolved.to_string_lossy() == target;
    (is_linked_to_target, !is_linked_to_target)
}

#[cfg(feature = "dev")]
fn scan_dir_for_plugins(dir: std::path::PathBuf) -> Vec<DiscoveredPlugin> {
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return vec![],
    };

    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| try_parse_plugin_dir(e.path()))
        .collect()
}

#[cfg(feature = "dev")]
fn try_parse_plugin_dir(path: std::path::PathBuf) -> Option<DiscoveredPlugin> {
    if !path.is_dir() {
        return None;
    }

    let plugin_toml = path.join("plugin.toml");
    if !plugin_toml.exists() {
        return None;
    }

    let id = path.file_name()?.to_string_lossy().to_string();
    if !id.starts_with("plugin-") || id == "plugin-template" {
        return None;
    }

    let name = read_plugin_name(&plugin_toml).unwrap_or_else(|| id.clone());

    Some(DiscoveredPlugin {
        id,
        name,
        path: path.to_string_lossy().to_string(),
        already_linked: false,
        installed_not_linked: false,
    })
}

#[cfg(feature = "dev")]
fn read_plugin_name(toml_path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(toml_path).ok()?;
    let manifest: crate::plugins::PluginManifest = toml::from_str(&content).ok()?;
    Some(manifest.plugin.name)
}

#[cfg(feature = "dev")]
fn get_plugin_search_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();

    if let Ok(cwd) = std::env::current_dir() {
        if let Some(parent) = cwd.parent() {
            dirs.push(parent.to_path_buf());
            add_subdirs(&mut dirs, parent);
        }
    }

    if let Some(home) = dirs::home_dir() {
        for base in ["Git", "Projects", "src", "dev"] {
            let path = home.join(base);
            dirs.push(path.clone());
            add_subdirs(&mut dirs, &path);
        }
    }

    dirs
}

#[cfg(feature = "dev")]
fn add_subdirs(dirs: &mut Vec<std::path::PathBuf>, parent: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(parent) else { return };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
}
