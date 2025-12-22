use anyhow::Result;
use axum::{routing::get, Json, Router};
use serde::Serialize;
use tower_http::cors::{Any, CorsLayer};

use crate::domain::config::ServerConfig;
use crate::utils;

const STATUS_PORT: u16 = 45460;

#[derive(Serialize)]
pub struct ServerStatus {
    hostname: String,
    ip: Option<String>,
    discovery_port: u16,
    command_port: u16,
    app_download_url: String,
}

pub async fn run() -> Result<()> {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any);

    let app = Router::new()
        .route("/status", get(get_status))
        .route("/health", get(health_check))
        .layer(cors);

    let addr = format!("127.0.0.1:{}", STATUS_PORT);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    log::info!("Status server listening on http://{}", addr);

    axum::serve(listener, app).await?;

    Ok(())
}

async fn get_status() -> Json<ServerStatus> {
    Json(ServerStatus {
        hostname: utils::get_hostname(),
        ip: utils::get_local_ip().map(|ip| ip.to_string()),
        discovery_port: ServerConfig::DISCOVERY_PORT,
        command_port: ServerConfig::COMMAND_PORT,
        app_download_url: "https://github.com/qol-tools/pointZ/releases/latest".to_string(),
    })
}

async fn health_check() -> &'static str {
    "ok"
}

