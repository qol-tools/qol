mod restore;
mod state_store;

#[cfg(target_os = "linux")]
mod monitor_move;
#[cfg(target_os = "linux")]
mod scripts;
#[cfg(target_os = "linux")]
mod system;

#[cfg(target_os = "macos")]
mod macos;

use std::env;
use std::process::ExitCode;

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

    if let Err(error) = execute_action(&action, &store) {
        eprintln!("{error}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}

#[cfg(target_os = "linux")]
fn execute_action(action: &str, store: &FileMinimizedStateStore) -> Result<(), String> {
    use system::{run_cinnamon_eval, X11WindowSystem};
    let system = X11WindowSystem;
    match action {
        "snap-left" => run_cinnamon_eval(scripts::SNAP_LEFT_SCRIPT).map(|_| ()),
        "snap-right" => run_cinnamon_eval(scripts::SNAP_RIGHT_SCRIPT).map(|_| ()),
        "snap-bottom" => run_cinnamon_eval(scripts::SNAP_BOTTOM_SCRIPT).map(|_| ()),
        "maximize" => run_cinnamon_eval(scripts::MAXIMIZE_SCRIPT).map(|_| ()),
        "minimize" => restore::minimize_window(&system, store),
        "restore" => restore::restore_window(&system, store),
        "center" => run_cinnamon_eval(scripts::CENTER_SCRIPT).map(|_| ()),
        "move-monitor-left" => monitor_move::move_monitor(scripts::MOVE_MONITOR_LEFT_SCRIPT),
        "move-monitor-right" => monitor_move::move_monitor(scripts::MOVE_MONITOR_RIGHT_SCRIPT),
        _ => Err(format!("Unknown action: {action}")),
    }
}

#[cfg(target_os = "macos")]
fn execute_action(action: &str, store: &FileMinimizedStateStore) -> Result<(), String> {
    let system = macos::MacWindowSystem;
    match action {
        "snap-left" => macos::snap_left(),
        "snap-right" => macos::snap_right(),
        "snap-bottom" => macos::snap_bottom(),
        "maximize" => macos::maximize(),
        "minimize" => restore::minimize_window(&system, store),
        "restore" => restore::restore_window(&system, store),
        "center" => macos::center(),
        "move-monitor-left" => macos::move_monitor_left(),
        "move-monitor-right" => macos::move_monitor_right(),
        _ => Err(format!("Unknown action: {action}")),
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
