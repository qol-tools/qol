use super::plugin_ui;

use std::net::SocketAddr;
use std::path::PathBuf;
use axum::{
    extract::{Path, ws::{WebSocketUpgrade, WebSocket}},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use tokio::sync::oneshot;
use tower_http::services::ServeDir;
use anyhow::Result;

use crate::plugins::PluginLoader;

#[derive(Serialize)]
struct PluginInfo {
    id: String,
    name: String,
    description: String,
    version: String,
    installed: bool,
}

#[derive(Serialize)]
struct UninstallResult {
    success: bool,
    message: String,
}

pub struct UiServerHandle {
    pub addr: SocketAddr,
    #[allow(dead_code)]
    shutdown_tx: oneshot::Sender<()>,
}

pub async fn start_ui_server(static_dir: &str) -> Result<UiServerHandle> {
    let plugins_dir = PluginLoader::default_plugin_dir()
        .unwrap_or_else(|_| PathBuf::from("~/.config/qol-tray/plugins"));

    let api = Router::new()
        .route("/plugins", get(list_plugins))
        .route("/install/:id", post(install_plugin))
        .route("/uninstall/:id", post(uninstall_plugin))
        .route("/ws/install/:id", get(install_ws));

    let app = Router::new()
        .nest("/api", api)
        .nest("/plugins", plugin_ui::router(plugins_dir))
        .fallback_service(ServeDir::new(static_dir));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:42700").await?;
    let addr = listener.local_addr()?;

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .ok();
    });

    Ok(UiServerHandle { addr, shutdown_tx })
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

async fn list_plugins() -> Json<Vec<PluginInfo>> {
    use super::github::GitHubClient;

    log::info!("API /plugins called");

    let client = GitHubClient::new("qol-tools");
    let plugins_dir = PluginLoader::default_plugin_dir()
        .unwrap_or_else(|_| PathBuf::from("~/.config/qol-tray/plugins"));

    let installed_plugins = get_installed_plugin_ids(&plugins_dir);

    log::info!("Fetching plugins from GitHub...");
    match client.list_plugins().await {
        Ok(metadata_list) => {
            log::info!("Found {} plugins from GitHub", metadata_list.len());
            let plugins = metadata_list
                .into_iter()
                .map(|m| PluginInfo {
                    id: m.id.clone(),
                    name: m.name,
                    description: m.description,
                    version: m.version,
                    installed: installed_plugins.contains(&m.id),
                })
                .collect();
            Json(plugins)
        }
        Err(e) => {
            log::error!("Failed to fetch plugins from GitHub: {}", e);
            Json(vec![])
        }
    }
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
