mod domain;
mod features;
mod input;
mod utils;
mod status_server;

use anyhow::Result;

use crate::features::discovery::discovery_service::DiscoveryService;
use crate::features::command::command_service::CommandService;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    log::info!("Starting PointZerver (headless mode)...");
    if let Some(ts) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.metadata().ok())
        .and_then(|meta| meta.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
    {
        let dt = chrono::DateTime::from_timestamp(ts.as_secs() as i64, 0)
            .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
            .unwrap_or_else(|| ts.as_secs().to_string());
        log::debug!("Binary built: {}", dt);
    }

    let input_handler = input::InputHandler::new()?;
    let discovery_service = DiscoveryService::new().await?;
    let command_service = CommandService::new(input_handler)?;

    spawn_discovery_service(discovery_service);
    spawn_status_server();

    log::info!("PointZerver ready - discovery and command services running");

    tokio::task::spawn_blocking(move || command_service.run_blocking())
        .await
        .map_err(|e| anyhow::anyhow!("Task join error: {}", e))??;

    Ok(())
}

fn spawn_discovery_service(discovery_service: DiscoveryService) {
    tokio::spawn(async move {
        if let Err(e) = discovery_service.run().await {
            log::error!("Discovery loop error: {}", e);
        }
    });
}

fn spawn_status_server() {
    tokio::spawn(async move {
        if let Err(e) = status_server::run().await {
            log::error!("Status server error: {}", e);
        }
    });
}
