use anyhow::Result;
use std::sync::Arc;

mod domain;
mod features;
mod input;
mod status_server;
mod utils;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--kill") {
        std::process::exit(0);
    }

    let config = Arc::new(domain::config::ServerConfig::load()?);
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let action = if args.iter().any(|a| a == "--action") {
        args.iter().skip_while(|a| *a != "--action").nth(1).cloned()
    } else if let Some(first) = args.first() {
        Some(first.clone())
    } else {
        None
    };

    if let Some(act) = action {
        log::info!("Handling action: {}", act);
    }

    let command_service = features::command::command_service::CommandService::new();
    let discovery_service =
        features::discovery::discovery_service::DiscoveryService::new(config.clone());

    tokio::select! {
        res = status_server::run() => res?,
        res = discovery_service.run() => res?,
        res = command_service.run() => res?,
    }

    Ok(())
}
