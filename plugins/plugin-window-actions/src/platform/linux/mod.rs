mod doctor;
mod glide;
mod monitor_move;
mod scripts;
mod system;

use std::env;
use std::path::PathBuf;

use crate::config::WindowActionsConfig;
use crate::restore;
use crate::restore::state_store::{FileMinimizedStateStore, LAST_MINIMIZED_WINDOW_FILE_NAME};

use system::{run_cinnamon_eval, X11WindowSystem};

pub(crate) use doctor::{platform_supported_check, required_binaries_check};
pub(crate) use glide::GlideController;

pub(crate) fn execute_action(
    action: &str,
    store: &FileMinimizedStateStore,
    config: &WindowActionsConfig,
) -> Result<(), String> {
    let system = X11WindowSystem;
    match action {
        "snap-left" => {
            run_cinnamon_eval(&scripts::snap_left_script(config.snap_fraction)).map(|_| ())
        }
        "snap-right" => {
            run_cinnamon_eval(&scripts::snap_right_script(config.snap_fraction)).map(|_| ())
        }
        "snap-bottom" => {
            run_cinnamon_eval(&scripts::snap_bottom_script(config.snap_fraction)).map(|_| ())
        }
        "maximize" => run_cinnamon_eval(scripts::MAXIMIZE_SCRIPT).map(|_| ()),
        "minimize" => restore::minimize_window(&system, store),
        "restore" => restore::restore_window(&system, store),
        "center" => run_cinnamon_eval(&scripts::center_script(config)).map(|_| ()),
        "move-monitor-left" => monitor_move::move_monitor(
            scripts::MOVE_MONITOR_LEFT_SCRIPT,
            config.reveal_taskbar_after_move,
        ),
        "move-monitor-right" => monitor_move::move_monitor(
            scripts::MOVE_MONITOR_RIGHT_SCRIPT,
            config.reveal_taskbar_after_move,
        ),
        _ => Err(format!("Unknown action: {action}")),
    }
}

pub(crate) fn state_file_path() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join(LAST_MINIMIZED_WINDOW_FILE_NAME)
}
