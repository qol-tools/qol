mod monitor_move;
mod restore;
mod scripts;
mod state_store;
mod system;

use std::env;
use std::process::ExitCode;

use state_store::{default_state_file_path, FileMinimizedStateStore};
use system::{run_cinnamon_eval, X11WindowSystem};

fn main() -> ExitCode {
    let action = match env::args().nth(1) {
        Some(action) => action,
        None => {
            eprintln!("Usage: window-actions <action>");
            return ExitCode::from(1);
        }
    };

    let system = X11WindowSystem;
    let store = FileMinimizedStateStore::new(default_state_file_path());

    if let Err(error) = execute_action(&action, &system, &store) {
        eprintln!("{error}");
        return ExitCode::from(1);
    }

    ExitCode::SUCCESS
}

fn execute_action(
    action: &str,
    system: &X11WindowSystem,
    store: &FileMinimizedStateStore,
) -> Result<(), String> {
    match action {
        "snap-left" => run_cinnamon_eval(scripts::SNAP_LEFT_SCRIPT).map(|_| ()),
        "snap-right" => run_cinnamon_eval(scripts::SNAP_RIGHT_SCRIPT).map(|_| ()),
        "snap-bottom" => run_cinnamon_eval(scripts::SNAP_BOTTOM_SCRIPT).map(|_| ()),
        "maximize" => run_cinnamon_eval(scripts::MAXIMIZE_SCRIPT).map(|_| ()),
        "minimize" => restore::minimize_window(system, store),
        "restore" => restore::restore_window(system, store),
        "center" => run_cinnamon_eval(scripts::CENTER_SCRIPT).map(|_| ()),
        "move-monitor-left" => monitor_move::move_monitor(scripts::MOVE_MONITOR_LEFT_SCRIPT),
        "move-monitor-right" => monitor_move::move_monitor(scripts::MOVE_MONITOR_RIGHT_SCRIPT),
        _ => Err(format!("Unknown action: {action}")),
    }
}
