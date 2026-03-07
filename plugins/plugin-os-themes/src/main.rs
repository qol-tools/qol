mod cursor;
mod daemon;
mod theme;

use std::env;
use std::process::ExitCode;
use std::sync::mpsc;

fn main() -> ExitCode {
    match env::args().nth(1).as_deref() {
        None | Some("run") => run_daemon(),
        Some("settings") => run_settings(),
        Some("kill") => {
            daemon::send_kill();
            ExitCode::SUCCESS
        }
        Some(action) => {
            eprintln!("Unknown action: {action}");
            ExitCode::from(1)
        }
    }
}

fn run_daemon() -> ExitCode {
    if daemon::send_ping() {
        return ExitCode::SUCCESS;
    }
    let (tx, rx) = mpsc::channel();
    if !daemon::start_listener(tx) {
        return ExitCode::from(1);
    }
    std::thread::spawn(move || {
        if matches!(rx.recv(), Ok(daemon::Command::Kill)) {
            cursor::request_shutdown();
        }
    });
    let result = cursor::run();
    daemon::cleanup();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            ExitCode::from(1)
        }
    }
}

fn run_settings() -> ExitCode {
    match cursor::open_settings() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e:#}");
            ExitCode::from(1)
        }
    }
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
