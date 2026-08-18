mod doctor;

use std::path::PathBuf;

use qol_windowing::{WindowId, WindowOps, WindowRect};

use crate::config::WindowActionsConfig;
use crate::restore::state_store::{FileMinimizedStateStore, LAST_MINIMIZED_WINDOW_FILE_NAME};
use crate::restore::{self, WindowSystem};

pub(crate) use doctor::{permissions_check, platform_supported_check, required_binaries_check};

pub(crate) struct GlideController;

impl GlideController {
    pub(crate) fn connect() -> Result<Self, String> {
        Err(unsupported("continuous window movement"))
    }

    pub(crate) fn update(
        &mut self,
        _direction: crate::glide::Direction,
        _phase: crate::glide::Phase,
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

pub(crate) const DIAGNOSTIC_ACTIONS: &[crate::cli::ActionSpec] = &[];

pub(crate) fn execute_action(
    action: &str,
    store: &FileMinimizedStateStore,
    _config: &WindowActionsConfig,
) -> Result<(), String> {
    let system = UnsupportedWindowSystem;
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

struct UnsupportedWindowSystem;

impl WindowOps for UnsupportedWindowSystem {
    fn enumerate_windows(&self) -> Result<Vec<WindowId>, String> {
        Err(unsupported("window stacking lookup"))
    }

    fn window_geometry(&self, _window_id: &WindowId) -> Result<Option<WindowRect>, String> {
        Err(unsupported("window geometry lookup"))
    }

    fn move_resize(&self, _window_id: &WindowId, _rect: WindowRect) -> Result<(), String> {
        Err(unsupported("window geometry restore"))
    }

    fn focus_window(&self, _window_id: &WindowId) -> Result<bool, String> {
        Err(unsupported("window activation"))
    }

    fn minimize_window(&self, _window_id: &WindowId) -> Result<bool, String> {
        Err(unsupported("minimize"))
    }

    fn restore_window(&self, _window_id: &WindowId) -> Result<bool, String> {
        Err(unsupported("window restore"))
    }

    fn active_window_id(&self) -> Result<Option<WindowId>, String> {
        Err(unsupported("active window lookup"))
    }
}

impl WindowSystem for UnsupportedWindowSystem {
    fn is_excluded_window_type(&self, _window_id: &WindowId) -> Result<bool, String> {
        Err(unsupported("window type lookup"))
    }

    fn is_hidden_window(&self, _window_id: &WindowId) -> Result<bool, String> {
        Err(unsupported("hidden window lookup"))
    }

    fn is_launcher_window(&self, _window_id: &WindowId) -> bool {
        false
    }

    fn window_pid(&self, _window_id: &WindowId) -> Result<Option<u32>, String> {
        Err(unsupported("window process lookup"))
    }

    fn process_start_ticks(&self, _pid: u32) -> Option<u64> {
        None
    }
}

fn unsupported(operation: &str) -> String {
    format!(
        "window-actions: {operation} is not implemented on {}",
        std::env::consts::OS
    )
}
