mod config;
mod platform;
mod restore;
mod state_store;

use std::env;
use std::process::ExitCode;

use config::load_config;
use state_store::{default_state_file_path, FileMinimizedStateStore};

fn main() -> ExitCode {
    let action = match env::args().nth(1) {
        Some(action) => action,
        None => {
            eprintln!("Usage: window-actions <action>");
            return ExitCode::from(1);
        }
    };

    let store = FileMinimizedStateStore::new(default_state_file_path());
    let config = load_config();

    if let Err(error) = platform::execute_action(&action, &store, &config) {
        eprintln!("{error}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use qol_tray::plugins::manifest::PluginManifest;

    #[test]
    fn validate_plugin_contract() {
        PluginManifest::load_and_validate("plugin.toml").expect("plugin.toml invalid");
    }
}
