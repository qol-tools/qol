mod doctor;
mod glide;

use std::path::PathBuf;

use qol_windowing::{WindowId, WindowRect};

use crate::config::WindowActionsConfig;
use crate::platform::layout;
use crate::restore::state_store::{FileMinimizedStateStore, LAST_MINIMIZED_WINDOW_FILE_NAME};
use crate::restore::{self, WindowSystem};

use qol_windowing::platform::windows::{self as win32, Win32Windows, Window};

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
        "minimize" => restore::minimize_window(&Win32Windows, store),
        "restore" => restore::restore_window(&Win32Windows, store),
        "move-monitor-left" => move_monitor(-1),
        "move-monitor-right" => move_monitor(1),
        _ => Err(format!("Unknown action: {action}")),
    }
}

pub(crate) fn state_file_path() -> PathBuf {
    std::env::temp_dir().join(LAST_MINIMIZED_WINDOW_FILE_NAME)
}

fn foreground() -> Result<Window, String> {
    Window::foreground()
        .ok_or_else(|| "No foreground window".to_string())
        .and_then(responsive)
}

fn responsive(window: Window) -> Result<Window, String> {
    if !window.is_hung() {
        return Ok(window);
    }
    let message = hung_refusal(&window.title());
    qol_plugin_daemon::notification::send_notification("Window Actions", &message);
    Err(message)
}

fn hung_refusal(title: &str) -> String {
    let title = title.trim();
    let window = if title.is_empty() {
        "This window".to_string()
    } else {
        format!("\"{title}\"")
    };
    format!("{window} is not responding, so Window Actions left it alone.")
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
    let target =
        layout::moved_to_monitor(frame, &screens, delta).map_err(|error| error.to_string())?;
    window.set_frame(target)?;
    if maximized {
        window.maximize();
    }
    Ok(())
}

impl WindowSystem for Win32Windows {
    fn is_excluded_window_type(&self, window_id: &WindowId) -> Result<bool, String> {
        Ok(!Window::try_from_id(window_id)?.is_switchable())
    }

    fn is_hidden_window(&self, window_id: &WindowId) -> Result<bool, String> {
        Ok(Window::try_from_id(window_id)?.is_minimized())
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
        Ok(Window::try_from_id(window_id)?.pid())
    }

    fn process_start_ticks(&self, pid: u32) -> Option<u64> {
        qol_process::process_start_time(pid)
    }

    fn ensure_responsive(&self, window_id: &WindowId) -> Result<(), String> {
        responsive(Window::try_from_id(window_id)?).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hung_window_is_refused_by_name() {
        let cases = [
            (
                "Untitled - Notepad",
                "\"Untitled - Notepad\" is not responding, so Window Actions left it alone.",
            ),
            (
                "  ",
                "This window is not responding, so Window Actions left it alone.",
            ),
        ];
        for (title, message) in cases {
            assert_eq!(hung_refusal(title), message, "{title:?}");
        }
    }
}
