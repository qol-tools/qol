use super::plugin_ui;

use std::path::PathBuf;
use axum::{
    extract::{Path, ws::{WebSocketUpgrade, WebSocket}},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
    http::{StatusCode, header},
};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer};
use axum::http::HeaderValue;
use anyhow::Result;

use crate::plugins::{PluginLoader, PluginManager};
use crate::hotkeys::trigger_reload;

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

pub struct UiServerHandle {
    #[allow(dead_code)]
    shutdown_tx: oneshot::Sender<()>,
}

pub async fn start_ui_server(static_dir: &str) -> Result<UiServerHandle> {
    let plugins_dir = PluginLoader::default_plugin_dir()
        .unwrap_or_else(|_| PathBuf::from("~/.config/qol-tray/plugins"));

    let plugins_dir_clone = plugins_dir.clone();
    let api = Router::new()
        .route("/plugins", get(list_plugins))
        .route("/installed", get(list_installed))
        .route("/cover/:id", get(serve_cover))
        .route("/install/:id", post(install_plugin))
        .route("/update/:id", post(update_plugin))
        .route("/uninstall/:id", post(uninstall_plugin))
        .route("/plugins/:id/config", get(get_plugin_config))
        .route("/plugins/:id/config", axum::routing::put(set_plugin_config))
        .route("/ws/install/:id", get(install_ws))
        .route("/github-token", get(get_token_status))
        .route("/github-token", post(set_github_token))
        .route("/github-token", axum::routing::delete(delete_github_token))
        .route("/hotkeys", get(get_hotkeys))
        .route("/hotkeys", axum::routing::put(set_hotkeys))
        .with_state(plugins_dir_clone);

    let static_service = ServeDir::new(static_dir);
    let no_cache = SetResponseHeaderLayer::overriding(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-store, must-revalidate"),
    );

    let app = Router::new()
        .nest("/api", api)
        .nest("/plugins", plugin_ui::router(plugins_dir))
        .fallback_service(static_service)
        .layer(no_cache);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:42700").await?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .ok();
    });

    Ok(UiServerHandle { shutdown_tx })
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
    let plugins_dir = PluginLoader::default_plugin_dir()
        .unwrap_or_else(|_| PathBuf::from("~/.config/qol-tray/plugins"));

    let installed_plugins = get_installed_plugin_ids(&plugins_dir);

    let cache_age = cache_age_secs();
    
    let plugins = match client.list_plugins_cached(query.refresh).await {
        Ok(metadata_list) => {
            log::info!("Got {} plugins", metadata_list.len());
            metadata_list
                .into_iter()
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

async fn install_plugin(Path(id): Path<String>) -> Json<PluginInfo> {
    use super::installer::PluginInstaller;

    log::info!("Install requested for plugin: {}", id);

    let plugins_dir = match PluginLoader::ensure_plugin_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("Failed to get plugins directory: {}", e);
            return Json(PluginInfo {
                id: id.clone(),
                name: id.clone(),
                description: format!("Failed to access plugins directory: {}", e),
                version: "0.0.0".to_string(),
                installed: false,
            });
        }
    };

    let installer = PluginInstaller::new(plugins_dir);
    let repo_url = format!("https://github.com/qol-tools/{}.git", id);

    if let Err(e) = installer.install(&repo_url, &id).await {
        log::error!("Failed to install plugin {}: {}", id, e);
        return Json(PluginInfo {
            id: id.clone(),
            name: id.clone(),
            description: format!("Installation failed: {}", e),
            version: "1.0.0".to_string(),
            installed: false,
        });
    }

    log::info!("Plugin {} installed successfully", id);
    Json(PluginInfo {
        id: id.clone(),
        name: id.clone(),
        description: "Installed successfully".to_string(),
        version: "1.0.0".to_string(),
        installed: true,
    })
}

async fn update_plugin(Path(id): Path<String>) -> Json<UninstallResult> {
    use super::installer::PluginInstaller;

    log::info!("Update requested for plugin: {}", id);

    let plugins_dir = match PluginLoader::default_plugin_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("Failed to get plugins directory: {}", e);
            return Json(UninstallResult {
                success: false,
                message: format!("Failed to access plugins directory: {}", e),
            });
        }
    };

    let installer = PluginInstaller::new(plugins_dir);

    if let Err(e) = installer.update(&id).await {
        log::error!("Failed to update plugin {}: {}", id, e);
        return Json(UninstallResult {
            success: false,
            message: format!("Update failed: {}", e),
        });
    }

    log::info!("Plugin {} updated successfully", id);
    Json(UninstallResult {
        success: true,
        message: "Updated successfully".to_string(),
    })
}

async fn uninstall_plugin(Path(id): Path<String>) -> Json<UninstallResult> {
    use super::installer::PluginInstaller;

    log::info!("Uninstall requested for plugin: {}", id);

    let plugins_dir = match PluginLoader::default_plugin_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("Failed to get plugins directory: {}", e);
            return Json(UninstallResult {
                success: false,
                message: format!("Failed to access plugins directory: {}", e),
            });
        }
    };

    let installer = PluginInstaller::new(plugins_dir);

    if let Err(e) = installer.uninstall(&id).await {
        log::error!("Failed to uninstall plugin {}: {}", id, e);
        return Json(UninstallResult {
            success: false,
            message: format!("Uninstall failed: {}", e),
        });
    }

    log::info!("Plugin {} uninstalled successfully", id);
    Json(UninstallResult {
        success: true,
        message: "Uninstalled successfully".to_string(),
    })
}

async fn install_ws(ws: WebSocketUpgrade, Path(id): Path<String>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| install_progress_socket(socket, id))
}

async fn install_progress_socket(mut socket: WebSocket, id: String) {
    use axum::extract::ws::Message;
    let _ = socket
        .send(Message::Text(format!("Starting install for {}", id)))
        .await;
}

async fn list_installed(
    axum::extract::State(_plugins_dir): axum::extract::State<PathBuf>,
) -> Json<Vec<InstalledPlugin>> {
    use super::github::read_cache;
    use std::collections::HashMap;

    let mut manager = PluginManager::new();

    if let Err(e) = manager.load_plugins() {
        log::error!("Failed to load installed plugins: {}", e);
        return Json(vec![]);
    }

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

    Json(plugins)
}

fn extract_actions(items: &[crate::plugins::MenuItem]) -> Vec<PluginAction> {
    use crate::plugins::MenuItem;
    let mut actions = Vec::new();
    
    for item in items {
        match item {
            MenuItem::Action { id, label, .. } => {
                actions.push(PluginAction {
                    id: id.clone(),
                    label: label.clone(),
                });
            }
            MenuItem::Submenu { items, .. } => {
                actions.extend(extract_actions(items));
            }
            _ => {}
        }
    }
    
    actions
}

fn is_newer_version(available: &str, installed: &str) -> bool {
    let parse = |s: &str| -> Vec<u32> {
        s.trim_start_matches('v')
            .split('.')
            .filter_map(|p| p.parse().ok())
            .collect()
    };
    
    let av = parse(available);
    let iv = parse(installed);
    
    for i in 0..av.len().max(iv.len()) {
        let a = av.get(i).copied().unwrap_or(0);
        let b = iv.get(i).copied().unwrap_or(0);
        if a > b { return true; }
        if a < b { return false; }
    }
    false
}

async fn serve_cover(
    Path(plugin_id): Path<String>,
    axum::extract::State(plugins_dir): axum::extract::State<PathBuf>,
) -> impl IntoResponse {
    let cover_path = plugins_dir.join(&plugin_id).join("cover.png");
    
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

    (StatusCode::OK, [(header::CONTENT_TYPE, "image/png")], data).into_response()
}

async fn get_plugin_config(
    Path(plugin_id): Path<String>,
    axum::extract::State(plugins_dir): axum::extract::State<PathBuf>,
) -> impl IntoResponse {
    let config_path = plugins_dir.join(&plugin_id).join("config.json");

    if !config_path.exists() {
        return (StatusCode::NOT_FOUND, "Config not found").into_response();
    }

    let data = match tokio::fs::read(&config_path).await {
        Ok(data) => data,
        Err(e) => {
            log::error!("Failed to read config: {}", e);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read config").into_response();
        }
    };

    (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], data).into_response()
}

async fn set_plugin_config(
    Path(plugin_id): Path<String>,
    axum::extract::State(plugins_dir): axum::extract::State<PathBuf>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let config_path = plugins_dir.join(&plugin_id).join("config.json");
    let plugin_dir = plugins_dir.join(&plugin_id);

    if !plugin_dir.exists() {
        return (StatusCode::NOT_FOUND, "Plugin not found").into_response();
    }

    if let Err(e) = serde_json::from_slice::<serde_json::Value>(&body) {
        log::error!("Invalid JSON in config: {}", e);
        return (StatusCode::BAD_REQUEST, "Invalid JSON").into_response();
    }

    if let Err(e) = tokio::fs::write(&config_path, &body).await {
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
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to store token: {}", e)).into_response();
    }

    log::info!("GitHub token stored successfully");
    (StatusCode::OK, "Token stored".to_string()).into_response()
}

async fn delete_github_token() -> impl IntoResponse {
    if let Err(e) = super::github::delete_token() {
        log::error!("Failed to delete GitHub token: {}", e);
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to delete token: {}", e)).into_response();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_version_returns_true_when_major_is_higher() {
        // Arrange
        let available = "2.0.0";
        let installed = "1.0.0";

        // Act
        let result = is_newer_version(available, installed);

        // Assert
        assert!(result);
    }

    #[test]
    fn is_newer_version_returns_true_when_minor_is_higher() {
        // Arrange
        let available = "1.2.0";
        let installed = "1.1.0";

        // Act
        let result = is_newer_version(available, installed);

        // Assert
        assert!(result);
    }

    #[test]
    fn is_newer_version_returns_true_when_patch_is_higher() {
        // Arrange
        let available = "1.0.5";
        let installed = "1.0.4";

        // Act
        let result = is_newer_version(available, installed);

        // Assert
        assert!(result);
    }

    #[test]
    fn is_newer_version_returns_false_when_versions_are_equal() {
        // Arrange
        let available = "1.2.3";
        let installed = "1.2.3";

        // Act
        let result = is_newer_version(available, installed);

        // Assert
        assert!(!result);
    }

    #[test]
    fn is_newer_version_returns_false_when_installed_is_newer() {
        // Arrange
        let available = "1.0.0";
        let installed = "2.0.0";

        // Act
        let result = is_newer_version(available, installed);

        // Assert
        assert!(!result);
    }

    #[test]
    fn is_newer_version_handles_v_prefix() {
        // Arrange
        let available = "v2.0.0";
        let installed = "v1.0.0";

        // Act
        let result = is_newer_version(available, installed);

        // Assert
        assert!(result);
    }

    #[test]
    fn is_newer_version_handles_mixed_v_prefix() {
        // Arrange
        let available = "v1.5.0";
        let installed = "1.4.0";

        // Act
        let result = is_newer_version(available, installed);

        // Assert
        assert!(result);
    }

    #[test]
    fn is_newer_version_handles_different_segment_counts() {
        // Arrange
        let available = "1.0.0.1";
        let installed = "1.0.0";

        // Act
        let result = is_newer_version(available, installed);

        // Assert
        assert!(result);
    }

    #[test]
    fn is_newer_version_returns_false_for_shorter_equal_version() {
        // Arrange
        let available = "1.0";
        let installed = "1.0.0";

        // Act
        let result = is_newer_version(available, installed);

        // Assert
        assert!(!result);
    }

    #[test]
    fn is_newer_version_handles_two_segment_versions() {
        // Arrange
        let available = "1.5";
        let installed = "1.4";

        // Act
        let result = is_newer_version(available, installed);

        // Assert
        assert!(result);
    }
}
