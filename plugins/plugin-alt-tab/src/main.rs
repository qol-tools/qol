mod app;
mod config;
mod daemon;
mod delegate;
mod icon;
mod layout;
mod monitor;
mod picker;
mod platform;
mod preview;
mod window_source;

use crate::config::load_alt_tab_config;
use std::sync::mpsc;

const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-alt-tab/";

fn maybe_open_settings(args: &[String]) -> bool {
    if !args.iter().any(|arg| arg == "--settings") {
        return false;
    }
    if let Err(error) = open::that(SETTINGS_URL) {
        eprintln!("Failed to open settings page: {}", error);
    }
    true
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if maybe_open_settings(&args) {
        return;
    }

    let is_show = args.iter().any(|a| a == "--show");
    let is_show_reverse = args.iter().any(|a| a == "--show-reverse");
    let is_kill = args.iter().any(|a| a == "--kill");

    if is_kill {
        daemon::send_kill();
        return;
    }

    // If daemon is alive, forward command and exit
    if is_show_reverse && daemon::send_show_reverse() {
        return;
    }
    if is_show && daemon::send_show() {
        return;
    }

    // Otherwise start as daemon
    let config = load_alt_tab_config();
    let (tx, rx) = mpsc::channel();

    if !daemon::start_listener(tx) {
        if is_show_reverse {
            daemon::send_show_reverse();
        } else if is_show {
            daemon::send_show();
        }
        return;
    }

    picker::run::run_app(config, rx, is_show || is_show_reverse);
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

        println!("Plugin contract passed successfully!");
    }
}
