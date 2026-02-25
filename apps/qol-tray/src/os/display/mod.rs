use qol_runtime::MonitorBounds;
use std::collections::HashSet;
use std::sync::{OnceLock, RwLock};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

static IGNORE_PIDS: OnceLock<RwLock<HashSet<u32>>> = OnceLock::new();

fn ignore_pids() -> &'static RwLock<HashSet<u32>> {
    IGNORE_PIDS.get_or_init(|| RwLock::new(HashSet::new()))
}

pub(crate) fn add_ignore_pid(pid: u32) {
    if let Ok(mut set) = ignore_pids().write() {
        set.insert(pid);
        log::debug!("[runtime/ignore_pids] ADD {} → set={:?}", pid, *set);
    }
}

pub(crate) fn remove_ignore_pid(pid: u32) {
    if let Ok(mut set) = ignore_pids().write() {
        set.remove(&pid);
        log::debug!("[runtime/ignore_pids] REMOVE {} → set={:?}", pid, *set);
    }
}

pub(crate) fn is_ignored_pid(pid: u32) -> bool {
    ignore_pids().read().map(|s| s.contains(&pid)).unwrap_or(false)
}

pub(crate) trait Platform: Send + Sync {
    fn cursor_position(&self) -> Option<(f32, f32)>;
    fn focused_window_bounds(&self) -> Option<MonitorBounds>;
    fn physical_monitors(&self) -> Vec<MonitorBounds>;

    fn poll_focused_window(&self) -> bool {
        true
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn create() -> impl Platform {
    linux::LinuxQueries::new()
}

#[cfg(target_os = "macos")]
pub(crate) fn create() -> impl Platform {
    macos::MacQueries::new(std::process::id() as i32)
}
