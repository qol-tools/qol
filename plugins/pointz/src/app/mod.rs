pub(crate) mod daemon;
pub(crate) mod pairing;

use crate::input::InputHandler;

pub(crate) fn run() {
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

    let (tx, rx) = std::sync::mpsc::channel();
    if !daemon::start_listener(tx) {
        if daemon::send_action("settings") {
            log::debug!("another instance running, sent settings");
        }
        return;
    }

    log::info!("daemon started");

    let input_handler = match InputHandler::new() {
        Ok(handler) => handler,
        Err(error) => {
            log::error!("Failed to create input handler: {}", error);
            daemon::cleanup();
            return;
        }
    };

    log::info!("PointZerver ready - qol-tray owns discovery, pairing and command authentication");

    for command in rx {
        match command {
            daemon::Command::Settings => crate::qol::open_settings(),
            daemon::Command::BeginPairing => pairing::begin(),
            daemon::Command::Input(input) => {
                match serde_json::from_value::<crate::command::Command>(input) {
                    Ok(command) => {
                        if let Err(error) = input_handler.handle_command(command) {
                            log::warn!("Rejected command: {error}");
                        }
                    }
                    Err(error) => log::warn!("Ignored malformed command: {error}"),
                }
            }
            daemon::Command::Kill => {
                log::info!("kill received, shutting down");
                daemon::cleanup();
                std::process::exit(0);
            }
        }
    }

    daemon::cleanup();
}
