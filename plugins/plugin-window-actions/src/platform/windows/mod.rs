use std::path::PathBuf;

use crate::config::WindowActionsConfig;
use crate::restore::{self, WindowSystem};
use crate::state_store::{FileMinimizedStateStore, LAST_MINIMIZED_WINDOW_FILE_NAME};

pub(crate) struct GlideController;

impl GlideController {
    pub(crate) fn connect() -> Result<Self, String> {
        Err(unsupported("continuous window movement"))
    }

    pub(crate) fn update(
        &mut self,
        _direction: crate::movement::Direction,
        _phase: crate::movement::Phase,
        _speed: f64,
    ) -> Result<String, String> {
        Err(unsupported("continuous window movement"))
    }

    pub(crate) fn stop_all(&mut self) -> Result<(), String> {
        Ok(())
    }

    pub(crate) fn maintain(&mut self) -> Option<Result<(), String>> {
        None
    }

    pub(crate) fn is_active(&self) -> bool {
        false
    }
}

pub(crate) fn execute_action(
    action: &str,
    store: &FileMinimizedStateStore,
    _config: &WindowActionsConfig,
) -> Result<(), String> {
    let system = WindowsWindowSystem;
    match action {
        "minimize" => restore::minimize_window(&system, store),
        "restore" => restore::restore_window(&system, store),
        "snap-left" | "snap-right" | "snap-bottom" | "maximize" | "center"
        | "move-monitor-left" | "move-monitor-right" => Err(unsupported(action)),
        _ => Err(format!("Unknown action: {action}")),
    }
}

pub(crate) fn state_file_path() -> PathBuf {
    std::env::temp_dir().join(LAST_MINIMIZED_WINDOW_FILE_NAME)
}

struct WindowsWindowSystem;

impl WindowSystem for WindowsWindowSystem {
    fn active_window_id(&self) -> Result<Option<String>, String> {
        Err(unsupported("active window lookup"))
    }

    fn minimize_window(&self, _window_id: &str) -> Result<bool, String> {
        Err(unsupported("minimize"))
    }

    fn window_rect(&self, _window_id: &str) -> Option<[f64; 4]> {
        None
    }

    fn stacking_window_ids(&self) -> Result<Vec<String>, String> {
        Err(unsupported("window stacking lookup"))
    }

    fn is_window_id(&self, _id: &str) -> bool {
        false
    }

    fn normalize_window_id(&self, _window_id: &str) -> Option<String> {
        None
    }

    fn is_excluded_window_type(&self, _window_id: &str) -> Result<bool, String> {
        Err(unsupported("window type lookup"))
    }

    fn is_hidden_window(&self, _window_id: &str) -> Result<bool, String> {
        Err(unsupported("hidden window lookup"))
    }

    fn is_launcher_window(&self, _window_id: &str) -> bool {
        false
    }

    fn activate_window(&self, _window_id: &str) -> Result<bool, String> {
        Err(unsupported("window activation"))
    }

    fn restore_rect(&self, _window_id: &str, _rect: [f64; 4]) -> Result<(), String> {
        Err(unsupported("window geometry restore"))
    }

    fn window_pid(&self, _window_id: &str) -> Result<Option<u32>, String> {
        Err(unsupported("window process lookup"))
    }

    fn process_start_ticks(&self, _pid: u32) -> Option<u64> {
        None
    }
}

fn unsupported(operation: &str) -> String {
    format!("window-actions: {operation} is not implemented on Windows")
}
