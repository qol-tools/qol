use qol_runtime::MonitorBounds;

use crate::desktop_state::Platform;

pub(super) struct FallbackQueries;

impl Platform for FallbackQueries {
    fn poll_focused_window(&self) -> bool {
        false
    }

    fn cursor_position(&self) -> Option<(f32, f32)> {
        None
    }

    fn focused_window_bounds(&self) -> Option<MonitorBounds> {
        None
    }

    fn physical_monitors(&self) -> Vec<MonitorBounds> {
        Vec::new()
    }
}
