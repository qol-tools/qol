mod doctor;
mod glide;
mod monitor_move;
mod scripts;
mod system;

use std::env;
use std::path::PathBuf;

use crate::action::WindowAction;
use crate::config::WindowActionsConfig;
use crate::restore;
use crate::restore::state_store::{FileMinimizedStateStore, LAST_MINIMIZED_WINDOW_FILE_NAME};

use system::{run_cinnamon_eval, X11WindowSystem};

pub(crate) use doctor::{platform_supported_check, required_binaries_check};
pub(crate) use glide::GlideController;

pub(crate) const DIAGNOSTIC_ACTIONS: &[WindowAction] = &[];

pub(crate) fn execute_action(
    action: WindowAction,
    store: &FileMinimizedStateStore,
    config: &WindowActionsConfig,
) -> Result<(), String> {
    let system = X11WindowSystem;
    match action {
        WindowAction::SnapLeft => {
            run_cinnamon_eval(&scripts::snap_left_script(config.snap_fraction)).map(|_| ())
        }
        WindowAction::SnapRight => {
            run_cinnamon_eval(&scripts::snap_right_script(config.snap_fraction)).map(|_| ())
        }
        WindowAction::SnapBottom => {
            run_cinnamon_eval(&scripts::snap_bottom_script(config.snap_fraction)).map(|_| ())
        }
        WindowAction::Maximize => run_cinnamon_eval(&scripts::maximize_script()).map(|_| ()),
        WindowAction::Minimize => restore::minimize_window(&system, store),
        WindowAction::Restore => restore::restore_window(&system, store),
        WindowAction::Center => run_cinnamon_eval(&scripts::center_script(config)).map(|_| ()),
        WindowAction::MoveMonitorLeft => monitor_move::move_monitor(
            &scripts::move_monitor_left_script(),
            config.reveal_taskbar_after_move,
        ),
        WindowAction::MoveMonitorRight => monitor_move::move_monitor(
            &scripts::move_monitor_right_script(),
            config.reveal_taskbar_after_move,
        ),
        WindowAction::Glide(_) | WindowAction::PrintWorkAreas => Err(action.unknown_message()),
    }
}

pub(crate) fn state_file_path() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join(LAST_MINIMIZED_WINDOW_FILE_NAME)
}
