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
struct InstalledPlugin {
    id: String,
    name: String,
    description: String,
    version: String,
    has_cover: bool,
    has_ui: bool,
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
        .route("/uninstall/:id", post(uninstall_plugin))
        .route("/ws/install/:id", get(install_ws))
        .route("/github-token", get(get_token_status))
        .route("/github-token", post(set_github_token))
        .route("/github-token", axum::routing::delete(delete_github_token))
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

    match installer.install(&repo_url, &id).await {
        Ok(_) => {
            log::info!("Plugin {} installed successfully", id);
            crate::tray::request_plugin_refresh();
            Json(PluginInfo {
                id: id.clone(),
                name: id.clone(),
                description: "Installed successfully".to_string(),
                version: "1.0.0".to_string(),
                installed: true,
            })
        }
        Err(e) => {
            log::error!("Failed to install plugin {}: {}", id, e);
            Json(PluginInfo {
                id: id.clone(),
                name: id.clone(),
                description: format!("Installation failed: {}", e),
                version: "1.0.0".to_string(),
                installed: false,
            })
        }
    }
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

    match installer.uninstall(&id).await {
        Ok(_) => {
            log::info!("Plugin {} uninstalled successfully", id);
            crate::tray::request_plugin_refresh();
            Json(UninstallResult {
                success: true,
                message: "Uninstalled successfully".to_string(),
            })
        }
        Err(e) => {
            log::error!("Failed to uninstall plugin {}: {}", id, e);
            Json(UninstallResult {
                success: false,
                message: format!("Uninstall failed: {}", e),
            })
        }
    }
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
    let mut manager = PluginManager::new();
    
    match manager.load_plugins() {
        Ok(_) => {
            let plugins: Vec<InstalledPlugin> = manager.plugins()
                .map(|plugin| {
                    let cover_path = plugin.path.join("cover.png");
                    let ui_path = plugin.path.join("ui").join("index.html");
                    
                    InstalledPlugin {
                        id: plugin.id.clone(),
                        name: plugin.manifest.plugin.name.clone(),
                        description: plugin.manifest.plugin.description.clone(),
                        version: plugin.manifest.plugin.version.clone(),
                        has_cover: cover_path.exists(),
                        has_ui: ui_path.exists(),
                    }
                })
                .collect();
            
            Json(plugins)
        }
        Err(e) => {
            log::error!("Failed to load installed plugins: {}", e);
            Json(vec![])
        }
    }
}

async fn serve_cover(
    Path(plugin_id): Path<String>,
    axum::extract::State(plugins_dir): axum::extract::State<PathBuf>,
) -> impl IntoResponse {
    let cover_path = plugins_dir.join(&plugin_id).join("cover.png");
    
    if !cover_path.exists() {
        return (StatusCode::NOT_FOUND, "Cover not found").into_response();
    }
    
    match tokio::fs::read(&cover_path).await {
        Ok(data) => {
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "image/png")],
                data,
            ).into_response()
        }
        Err(e) => {
            log::error!("Failed to read cover image: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "Failed to read cover").into_response()
        }
    }
}

async fn get_token_status() -> Json<TokenStatus> {
    Json(TokenStatus {
        has_token: super::github::get_stored_token().is_some(),
    })
}

async fn set_github_token(Json(payload): Json<TokenRequest>) -> impl IntoResponse {
    match super::github::store_token(&payload.token) {
        Ok(_) => {
            log::info!("GitHub token stored successfully");
            (StatusCode::OK, "Token stored").into_response()
        }
        Err(e) => {
            log::error!("Failed to store GitHub token: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to store token: {}", e)).into_response()
        }
    }
}

async fn delete_github_token() -> impl IntoResponse {
    match super::github::delete_token() {
        Ok(_) => {
            log::info!("GitHub token deleted");
            (StatusCode::OK, "Token deleted").into_response()
        }
        Err(e) => {
            log::error!("Failed to delete GitHub token: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to delete token: {}", e)).into_response()
        }
    }
}
