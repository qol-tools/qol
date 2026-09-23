use std::sync::Arc;

use qol_runtime::MonitorBounds;

mod ignore_pids;
mod platform;

#[cfg(all(target_os = "linux", feature = "linux_evdev"))]
pub(crate) use platform::is_wayland;

pub(crate) type SharedPlatform = Arc<dyn Platform>;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct FocusedWindow {
    pub id: Option<u32>,
    pub monitor: MonitorBounds,
}

pub(crate) trait Platform: Send + Sync {
    fn cursor_position(&self) -> Option<(f32, f32)>;
    fn focused_window_bounds(&self) -> Option<MonitorBounds>;
    fn physical_monitors(&self) -> Vec<MonitorBounds>;

    fn focused_window(&self) -> Option<FocusedWindow> {
        Some(FocusedWindow {
            id: None,
            monitor: self.focused_window_bounds()?,
        })
    }

    fn poll_focused_window(&self) -> bool {
        true
    }

    fn window_list_fingerprint(&self) -> Option<u64> {
        None
    }
}

pub(crate) fn create_shared() -> SharedPlatform {
    Arc::new(platform::create())
}

pub(crate) use ignore_pids::is_ignored_pid;
pub(crate) use ignore_pids::{add_ignore_pid, remove_ignore_pid};
