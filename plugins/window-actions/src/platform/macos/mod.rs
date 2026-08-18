mod ax;
mod doctor;
mod geometry;
mod objc;
mod screen;
mod trace;

use std::path::PathBuf;
use std::process::Command;

use qol_windowing::{WindowId, WindowOps, WindowRect};

use crate::config::WindowActionsConfig;
use crate::restore::state_store::{FileMinimizedStateStore, LAST_MINIMIZED_WINDOW_FILE_NAME};
use crate::restore::WindowSystem;

use geometry::{
    ax_set, center, maximize, move_monitor_left, move_monitor_right, snap_bottom, snap_left,
    snap_right,
};

pub(crate) use doctor::{permissions_check, platform_supported_check, required_binaries_check};

pub(crate) struct GlideController;

impl GlideController {
    pub(crate) fn connect() -> Result<Self, String> {
        Err("continuous window movement is not yet available on macOS".into())
    }

    pub(crate) fn update(
        &mut self,
        _direction: crate::glide::Direction,
        _phase: crate::glide::Phase,
        _speed: f64,
    ) -> Result<String, String> {
        Err("continuous window movement is not yet available on macOS".into())
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

pub(crate) const DIAGNOSTIC_ACTIONS: &[crate::cli::ActionSpec] =
    &[crate::cli::ActionSpec::ordinary(
        screen::SCREENS_ACTION,
        "Print each display's work area as x,y,w,h lines.",
    )];

pub(crate) fn execute_action(
    action: &str,
    store: &FileMinimizedStateStore,
    config: &WindowActionsConfig,
) -> Result<(), String> {
    let system = MacWindowSystem;
    match action {
        "snap-left" => snap_left(config),
        "snap-right" => snap_right(config),
        "snap-bottom" => snap_bottom(config),
        "maximize" => maximize(),
        "minimize" => crate::restore::minimize_window(&system, store),
        "restore" => crate::restore::restore_window(&system, store),
        "center" => center(config),
        "move-monitor-left" => move_monitor_left(),
        "move-monitor-right" => move_monitor_right(),
        screen::SCREENS_ACTION => {
            screen::print_work_areas();
            Ok(())
        }
        _ => Err(format!("Unknown action: {action}")),
    }
}

pub(crate) fn state_file_path() -> PathBuf {
    std::env::temp_dir().join(LAST_MINIMIZED_WINDOW_FILE_NAME)
}

pub(crate) struct MacWindowSystem;

impl WindowOps for MacWindowSystem {
    fn enumerate_windows(&self) -> Result<Vec<WindowId>, String> {
        Ok(vec![])
    }

    fn window_geometry(&self, window_id: &WindowId) -> Result<Option<WindowRect>, String> {
        let pid = parse_pid(window_id.as_str())
            .ok_or_else(|| format!("Invalid window ID: {}", window_id.as_str()))?
            as i32;
        let Some(r) = ax::front_window_rect(pid) else {
            return Ok(None);
        };
        Ok(Some(WindowRect {
            x: r.x,
            y: r.y,
            width: r.w,
            height: r.h,
        }))
    }

    fn move_resize(&self, window_id: &WindowId, rect: WindowRect) -> Result<(), String> {
        let pid = parse_pid(window_id.as_str())
            .ok_or_else(|| format!("Invalid window ID: {}", window_id.as_str()))?
            as i32;
        let rect = screen::Rect {
            x: rect.x,
            y: rect.y,
            w: rect.width,
            h: rect.height,
        };
        ax_set(pid, rect)
    }

    fn focus_window(&self, window_id: &WindowId) -> Result<bool, String> {
        let pid = parse_pid(window_id.as_str())
            .ok_or_else(|| format!("Invalid window ID: {}", window_id.as_str()))?;
        Ok(ax::unminimize_and_raise(pid as i32))
    }

    fn minimize_window(&self, window_id: &WindowId) -> Result<bool, String> {
        let pid = parse_pid(window_id.as_str())
            .ok_or_else(|| format!("Invalid window ID: {}", window_id.as_str()))?;
        Ok(ax::instant_minimize(pid as i32))
    }

    fn restore_window(&self, window_id: &WindowId) -> Result<bool, String> {
        self.focus_window(window_id)
    }

    fn active_window_id(&self) -> Result<Option<WindowId>, String> {
        let Some(pid) = ax::find_normal_window_pid().filter(|p| *p > 0) else {
            return Ok(None);
        };
        Ok(WindowId::parse(&format!("pid:{pid}")))
    }
}

impl WindowSystem for MacWindowSystem {
    fn is_excluded_window_type(&self, window_id: &WindowId) -> Result<bool, String> {
        let Some(pid) = parse_pid(window_id.as_str()) else {
            return Ok(true);
        };
        Ok(!ax::is_normal_window(pid as i32))
    }

    fn is_hidden_window(&self, _window_id: &WindowId) -> Result<bool, String> {
        Ok(true)
    }

    fn is_launcher_window(&self, window_id: &WindowId) -> bool {
        let Some(pid) = parse_pid(window_id.as_str()) else {
            return false;
        };
        process_name(pid as u32)
            .map(|name| {
                let lower = name.to_ascii_lowercase();
                qol_conventions::launcher::MATCH_MARKERS
                    .iter()
                    .any(|marker| lower.contains(marker))
            })
            .unwrap_or(false)
    }

    fn window_pid(&self, window_id: &WindowId) -> Result<Option<u32>, String> {
        Ok(parse_pid(window_id.as_str()).map(|p| p as u32))
    }

    fn process_start_ticks(&self, pid: u32) -> Option<u64> {
        trace::timed_opt("ps_lstart", pid as i32, || {
            let output = Command::new("ps")
                .args(["-o", "lstart=", "-p", &pid.to_string()])
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            let raw = String::from_utf8_lossy(&output.stdout);
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return None;
            }
            Some(fnv1a(trimmed.as_bytes()))
        })
    }
}

fn parse_pid(window_id: &str) -> Option<i64> {
    window_id.split(':').nth(1)?.parse().ok()
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn process_name(pid: u32) -> Option<String> {
    trace::timed_opt("ps_comm", pid as i32, || {
        let output = Command::new("ps")
            .args(["-o", "comm=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if name.is_empty() {
            return None;
        }
        Some(name)
    })
}
