use gpui::*;
use qol_runtime::{MonitorBounds, PlatformStateClient};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct ActiveMonitor {
    inner: MonitorBounds,
}

impl ActiveMonitor {
    fn from_bounds(b: MonitorBounds) -> Self {
        Self { inner: b }
    }

    pub fn centered_bounds(&self, win_size: Size<Pixels>) -> Bounds<Pixels> {
        let x = px(self.inner.x) + (px(self.inner.width) - win_size.width) / 2.0;
        let y = px(self.inner.y) + (px(self.inner.height) - win_size.height) / 3.0;
        Bounds::new(point(x, y), win_size)
    }

    pub fn size(&self) -> (f32, f32) {
        (self.inner.width, self.inner.height)
    }

    pub fn bounds(&self) -> Bounds<Pixels> {
        Bounds::new(
            point(px(self.inner.x), px(self.inner.y)),
            size(px(self.inner.width), px(self.inner.height)),
        )
    }
}

#[derive(Clone)]
pub struct MonitorTracker {
    client: PlatformStateClient,
    any_visible: Arc<AtomicBool>,
}

impl MonitorTracker {
    pub fn start(_cx: &App, any_visible: Arc<AtomicBool>) -> Self {
        Self {
            client: PlatformStateClient::from_env(),
            any_visible,
        }
    }

    /// Returns (ActiveMonitor, active_monitor_idx) from one GET_STATE call.
    pub fn snapshot(&self) -> Option<(ActiveMonitor, Option<usize>)> {
        let state = self.client.get_state()?;

        if state.monitors.is_empty() {
            return None;
        }
        if state.monitors.len() == 1 {
            return Some((ActiveMonitor::from_bounds(state.monitors[0]), Some(0)));
        }

        let monitor = state.active_monitor()
            .or_else(|| state.cursor_monitor())
            .unwrap_or(state.monitors[0]);

        #[cfg(debug_assertions)]
        eprintln!(
            "[alt-tab/monitor] snapshot: cursor_idx={:?} focus_idx={:?} active_idx={:?} → ({}, {})",
            state.cursor_monitor_idx,
            state.focus_monitor_idx,
            state.active_monitor_idx,
            monitor.x,
            monitor.y,
        );

        Some((ActiveMonitor::from_bounds(monitor), state.active_monitor_idx))
    }

}
