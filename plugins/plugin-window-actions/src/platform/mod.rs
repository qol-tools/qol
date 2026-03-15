#[cfg(target_os = "linux")]
mod monitor_move;
#[cfg(target_os = "linux")]
mod scripts;
#[cfg(target_os = "linux")]
mod system;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!(
    "plugin-window-actions: unsupported target OS; add src/platform/<os>.rs and wire it in src/platform/mod.rs"
);

use crate::config::WindowActionsConfig;
use crate::state_store::FileMinimizedStateStore;

#[cfg(target_os = "linux")]
pub fn execute_action(
    action: &str,
    store: &FileMinimizedStateStore,
    config: &WindowActionsConfig,
) -> Result<(), String> {
    use crate::restore;
    use system::{run_cinnamon_eval, X11WindowSystem};
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

#[cfg(target_os = "macos")]
pub fn execute_action(
    action: &str,
    store: &FileMinimizedStateStore,
    config: &WindowActionsConfig,
) -> Result<(), String> {
    use crate::restore;
    let system = macos::MacWindowSystem;
    match action {
        "snap-left" => macos::snap_left(config),
        "snap-right" => macos::snap_right(config),
        "snap-bottom" => macos::snap_bottom(config),
        "maximize" => macos::maximize(),
        "minimize" => restore::minimize_window(&system, store),
        "restore" => restore::restore_window(&system, store),
        "center" => macos::center(config),
        "move-monitor-left" => macos::move_monitor_left(),
        "move-monitor-right" => macos::move_monitor_right(),
        _ => Err(format!("Unknown action: {action}")),
    }
}
