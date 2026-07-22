mod ax;
mod geometry;
mod objc;
mod screen;
mod trace;

use std::path::PathBuf;
use std::process::Command;

use crate::config::WindowActionsConfig;
use crate::restore::state_store::{FileMinimizedStateStore, LAST_MINIMIZED_WINDOW_FILE_NAME};
use crate::restore::WindowSystem;

use geometry::{
    center, maximize, move_monitor_left, move_monitor_right, snap_bottom, snap_left, snap_right,
};

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
        _ => Err(format!("Unknown action: {action}")),
    }
}

pub(crate) fn state_file_path() -> PathBuf {
    std::env::temp_dir().join(LAST_MINIMIZED_WINDOW_FILE_NAME)
}

pub struct MacWindowSystem;

impl WindowSystem for MacWindowSystem {
    fn active_window_id(&self) -> Result<Option<String>, String> {
        let Some(pid) = ax::find_normal_window_pid().filter(|p| *p > 0) else {
            return Ok(None);
        };
        Ok(Some(format!("pid:{pid}")))
    }

    fn minimize_window(&self, window_id: &str) -> Result<bool, String> {
        let pid = parse_pid(window_id).ok_or_else(|| format!("Invalid window ID: {window_id}"))?;
        Ok(ax::instant_minimize(pid as i32))
    }

    fn window_rect(&self, window_id: &str) -> Option<[f64; 4]> {
        let pid = parse_pid(window_id)? as i32;
        let r = ax::front_window_rect(pid)?;
        Some([r.x, r.y, r.w, r.h])
    }

    fn stacking_window_ids(&self) -> Result<Vec<String>, String> {
        Ok(vec![])
    }

    fn is_window_id(&self, id: &str) -> bool {
        id.starts_with("pid:")
    }

    fn normalize_window_id(&self, window_id: &str) -> Option<String> {
        if !self.is_window_id(window_id) {
            return None;
        }
        Some(window_id.to_string())
    }

    fn is_excluded_window_type(&self, window_id: &str) -> Result<bool, String> {
        let Some(pid) = parse_pid(window_id) else {
            return Ok(true);
        };
        Ok(!ax::is_normal_window(pid as i32))
    }

    fn is_hidden_window(&self, _window_id: &str) -> Result<bool, String> {
        Ok(true)
    }

    fn is_launcher_window(&self, window_id: &str) -> bool {
        let Some(pid) = parse_pid(window_id) else {
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

    fn activate_window(&self, window_id: &str) -> Result<bool, String> {
        let pid = parse_pid(window_id).ok_or_else(|| format!("Invalid window ID: {window_id}"))?;
        Ok(ax::unminimize_and_raise(pid as i32))
    }

    fn restore_rect(&self, window_id: &str, rect: [f64; 4]) -> Result<(), String> {
        let pid = parse_pid(window_id).ok_or_else(|| format!("Invalid window ID: {window_id}"))?;
        let _ = (pid, rect);
        Ok(())
    }

    fn window_pid(&self, window_id: &str) -> Result<Option<u32>, String> {
        Ok(parse_pid(window_id).map(|p| p as u32))
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
