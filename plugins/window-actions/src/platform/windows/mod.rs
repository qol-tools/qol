mod doctor;
mod glide;

use std::path::PathBuf;

use qol_windowing::{WindowId, WindowOps, WindowRect};

use crate::config::WindowActionsConfig;
use crate::platform::layout;
use crate::restore::state_store::{FileMinimizedStateStore, LAST_MINIMIZED_WINDOW_FILE_NAME};
use crate::restore::{self, WindowSystem};

use qol_windowing::platform::windows::{self as win32, Window};

pub(crate) use doctor::{platform_supported_check, required_binaries_check};
pub(crate) use glide::GlideController;

pub(crate) const DIAGNOSTIC_ACTIONS: &[crate::cli::ActionSpec] = &[];

pub(crate) fn execute_action(
    action: &str,
    store: &FileMinimizedStateStore,
    config: &WindowActionsConfig,
) -> Result<(), String> {
    win32::ensure_dpi_awareness();
    let fraction = config.snap_fraction;
    match action {
        "snap-left" => place_foreground(|work| layout::snap_left(work, fraction)),
        "snap-right" => place_foreground(|work| layout::snap_right(work, fraction)),
        "snap-bottom" => place_foreground(|work| layout::snap_bottom(work, fraction)),
        "center" => place_foreground(|work| layout::centered(work, config)),
        "maximize" => foreground().map(Window::maximize),
        "minimize" => restore::minimize_window(&Win32WindowSystem, store),
        "restore" => restore::restore_window(&Win32WindowSystem, store),
        "move-monitor-left" => move_monitor(-1),
        "move-monitor-right" => move_monitor(1),
        _ => Err(format!("Unknown action: {action}")),
    }
}

pub(crate) fn state_file_path() -> PathBuf {
    std::env::temp_dir().join(LAST_MINIMIZED_WINDOW_FILE_NAME)
}

fn foreground() -> Result<Window, String> {
    Window::foreground().ok_or_else(|| "No foreground window".to_string())
}

fn place_foreground(target: impl FnOnce(WindowRect) -> WindowRect) -> Result<(), String> {
    let window = foreground()?;
    let work = window
        .work_area()
        .ok_or("Cannot read the monitor work area")?;
    window.set_frame(target(work))
}

fn move_monitor(delta: i32) -> Result<(), String> {
    let window = foreground()?;
    let maximized = window.is_maximized();
    let frame = window.frame().ok_or("Cannot read window geometry")?;
    let screens = win32::work_areas_left_to_right();
    let Some(target) = layout::moved_to_monitor(frame, &screens, delta) else {
        return Ok(());
    };
    window.set_frame(target)?;
    if maximized {
        window.maximize();
    }
    Ok(())
}

fn window(window_id: &WindowId) -> Result<Window, String> {
    Window::from_id(window_id).ok_or_else(|| format!("Invalid window ID: {}", window_id.as_str()))
}

struct Win32WindowSystem;

impl WindowOps for Win32WindowSystem {
    fn enumerate_windows(&self) -> Result<Vec<WindowId>, String> {
        Ok(win32::top_level_windows()
            .into_iter()
            .filter(|window| window.is_switchable())
            .map(Window::id)
            .collect())
    }

    fn window_geometry(&self, window_id: &WindowId) -> Result<Option<WindowRect>, String> {
        let window = window(window_id)?;
        if !window.exists() {
            return Ok(None);
        }
        Ok(window.frame())
    }

    fn move_resize(&self, window_id: &WindowId, rect: WindowRect) -> Result<(), String> {
        window(window_id)?.set_frame(rect)
    }

    fn focus_window(&self, window_id: &WindowId) -> Result<bool, String> {
        Ok(window(window_id)?.activate())
    }

    fn minimize_window(&self, window_id: &WindowId) -> Result<bool, String> {
        Ok(window(window_id)?.minimize())
    }

    fn restore_window(&self, window_id: &WindowId) -> Result<bool, String> {
        self.focus_window(window_id)
    }

    fn active_window_id(&self) -> Result<Option<WindowId>, String> {
        Ok(Window::foreground().map(Window::id))
    }
}

impl WindowSystem for Win32WindowSystem {
    fn is_excluded_window_type(&self, window_id: &WindowId) -> Result<bool, String> {
        Ok(!window(window_id)?.is_switchable())
    }

    fn is_hidden_window(&self, window_id: &WindowId) -> Result<bool, String> {
        Ok(window(window_id)?.is_minimized())
    }

    fn is_launcher_window(&self, window_id: &WindowId) -> bool {
        Window::from_id(window_id).is_some_and(|window| {
            let title = window.title().to_ascii_lowercase();
            qol_conventions::launcher::MATCH_MARKERS
                .iter()
                .any(|marker| title.contains(marker))
        })
    }

    fn window_pid(&self, window_id: &WindowId) -> Result<Option<u32>, String> {
        Ok(window(window_id)?.pid())
    }

    fn process_start_ticks(&self, pid: u32) -> Option<u64> {
        qol_app_icon::process_start_time_us(i32::try_from(pid).ok()?)
    }
}
