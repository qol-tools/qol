mod daemon;
mod domain;
mod features;
mod input;
mod platform;
mod utils;

use crate::features::command::command_service::CommandService;
use crate::features::discovery::discovery_service::DiscoveryService;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "kill") {
        if daemon::send_kill() {
            eprintln!("[pointz] kill sent");
        } else {
            eprintln!("[pointz] no daemon running");
        }
        return;
    }

    let action = if args.iter().any(|a| a == "--action") {
        args.iter().skip_while(|a| *a != "--action").nth(1).cloned()
    } else {
        args.first().cloned()
    };

    if let Some(action) = action {
        if daemon::send_action(&action) {
            eprintln!("[pointz] action '{}' sent", action);
        } else {
            eprintln!("[pointz] no daemon running, handling locally");
            if action == "settings" {
                platform::open_settings();
            }
        }
        return;
    }

    run_daemon();
}

#[tokio::main]
async fn run_daemon() {
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
            eprintln!("[pointz] another instance running, sent settings");
        }
        return;
    }

    eprintln!("[pointz] daemon started");

    std::thread::spawn(move || {
        for cmd in rx {
            match cmd {
                daemon::Command::Settings => {
                    platform::open_settings();
                }
                daemon::Command::Kill => {
                    eprintln!("[pointz] kill received, shutting down");
                    daemon::cleanup();
                    std::process::exit(0);
                }
            }
        }
    });

    let input_handler = match input::InputHandler::new() {
        Ok(h) => h,
        Err(e) => {
            log::error!("Failed to create input handler: {}", e);
            daemon::cleanup();
            return;
        }
    };

    let discovery_service = match DiscoveryService::new().await {
        Ok(d) => d,
        Err(e) => {
            log::error!("Failed to create discovery service: {}", e);
            daemon::cleanup();
            return;
        }
    };

    let command_service = match CommandService::new(input_handler) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to create command service: {}", e);
            daemon::cleanup();
            return;
        }
    };

    tokio::spawn(async move {
        if let Err(e) = discovery_service.run().await {
            log::error!("Discovery loop error: {}", e);
        }
    });

    log::info!("PointZerver ready - discovery and command services running");

    if let Err(e) = command_service.run().await {
        log::error!("Command service error: {}", e);
    }

    daemon::cleanup();
}

#[cfg(test)]
mod tests {
    use qol_tray::plugins::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        let manifest_str =
            std::fs::read_to_string("plugin.toml").expect("Failed to read plugin.toml");
        let manifest: PluginManifest =
            toml::from_str(&manifest_str).expect("Failed to parse plugin.toml");
        manifest.validate().expect("Manifest validation failed");
    }
}
