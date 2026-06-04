use std::sync::Arc;

use qol_runtime::MonitorBounds;

mod ignore_pids;
mod platform;

pub(crate) type SharedPlatform = Arc<dyn Platform>;

pub(crate) trait Platform: Send + Sync {
    fn cursor_position(&self) -> Option<(f32, f32)>;
    fn focused_window_bounds(&self) -> Option<MonitorBounds>;
    fn physical_monitors(&self) -> Vec<MonitorBounds>;

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

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) use ignore_pids::is_ignored_pid;
pub(crate) use ignore_pids::{add_ignore_pid, remove_ignore_pid};
