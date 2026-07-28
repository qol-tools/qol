pub(crate) mod daemon;

use crate::command::CommandService;
use crate::discovery::DiscoveryService;
use crate::input::InputHandler;
use crate::security::CommandGate;
use std::sync::Arc;

#[tokio::main]
pub(crate) async fn run() {
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

    let security = match CommandGate::load() {
        Ok(security) => Arc::new(security),
        Err(error) => {
            log::error!("Failed to initialize PointZ security: {}", error);
            return;
        }
    };

    let (tx, rx) = std::sync::mpsc::channel();
    if !daemon::start_listener(tx) {
        if daemon::send_action("settings") {
            eprintln!("[pointz] another instance running, sent settings");
        }
        return;
    }

    eprintln!("[pointz] daemon started");

    let daemon_security = Arc::clone(&security);
    std::thread::spawn(move || {
        for cmd in rx {
            match cmd {
                daemon::Command::Settings => {
                    crate::qol::open_settings();
                }
                daemon::Command::BeginPairing => {
                    daemon_security.begin_pairing();
                }
                daemon::Command::Kill => {
                    eprintln!("[pointz] kill received, shutting down");
                    daemon::cleanup();
                    std::process::exit(0);
                }
            }
        }
    });

    let input_handler = match InputHandler::new() {
        Ok(handler) => handler,
        Err(error) => {
            log::error!("Failed to create input handler: {}", error);
            daemon::cleanup();
            return;
        }
    };

    let discovery_service = match DiscoveryService::new(Arc::clone(&security)).await {
        Ok(service) => service,
        Err(error) => {
            log::error!("Failed to create discovery service: {}", error);
            daemon::cleanup();
            return;
        }
    };

    let command_service = match CommandService::new(input_handler, security) {
        Ok(service) => service,
        Err(error) => {
            log::error!("Failed to create command service: {}", error);
            daemon::cleanup();
            return;
        }
    };

    tokio::spawn(async move {
        if let Err(error) = discovery_service.run().await {
            log::error!("Discovery loop error: {}", error);
        }
    });

    log::info!("PointZerver ready - discovery and command services running");

    if let Err(error) = command_service.run().await {
        log::error!("Command service error: {}", error);
    }

    daemon::cleanup();
}
