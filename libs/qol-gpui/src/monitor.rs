use gpui::*;
use qol_runtime::{MonitorBounds, PlatformStateClient};

#[derive(Clone, Debug)]
pub struct ActiveMonitor {
    inner: MonitorBounds,
}

impl ActiveMonitor {
    pub fn from_bounds(b: MonitorBounds) -> Self {
        Self { inner: b }
    }

    pub fn from_event(event: &crate::protocol::RuntimeEvent) -> Option<Self> {
        use crate::protocol::RuntimeEvent;
        match event {
            RuntimeEvent::ActiveMonitorChanged { monitor, .. }
            | RuntimeEvent::FocusChanged { monitor, .. } => (*monitor).map(Self::from_bounds),
            RuntimeEvent::CursorMoved { .. }
            | RuntimeEvent::LauncherAppsSynced { .. }
            | RuntimeEvent::MonitorsChanged { .. }
            | RuntimeEvent::WindowListChanged => None,
        }
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
}

impl MonitorTracker {
    pub fn start(_cx: &App) -> Self {
        Self {
            client: PlatformStateClient::from_env(),
        }
    }

    pub fn snapshot_monitor(&self) -> Option<ActiveMonitor> {
        self.snapshot().map(|(monitor, _)| monitor)
    }

    pub fn snapshot_monitor_focus_first(&self) -> Option<ActiveMonitor> {
        let state = self.client.get_state()?;
        if state.monitors.is_empty() {
            return None;
        }
        let monitor = state
            .focus_monitor()
            .or_else(|| state.active_monitor())
            .or_else(|| state.cursor_monitor())
            .unwrap_or(state.monitors[0]);
        #[cfg(debug_assertions)]
        eprintln!(
            "[monitor] focus-first snapshot: cursor_idx={:?} focus_idx={:?} active_idx={:?} -> ({}, {})",
            state.cursor_monitor_idx,
            state.focus_monitor_idx,
            state.active_monitor_idx,
            monitor.x,
            monitor.y,
        );
        Some(ActiveMonitor::from_bounds(monitor))
    }

    pub fn snapshot(&self) -> Option<(ActiveMonitor, Option<usize>)> {
        let state = self.client.get_state()?;

        if state.monitors.is_empty() {
            return None;
        }
        if state.monitors.len() == 1 {
            return Some((ActiveMonitor::from_bounds(state.monitors[0]), Some(0)));
        }

        let monitor = state
            .active_monitor()
            .or_else(|| state.cursor_monitor())
            .unwrap_or(state.monitors[0]);

        #[cfg(debug_assertions)]
        eprintln!(
            "[monitor] snapshot: cursor_idx={:?} focus_idx={:?} active_idx={:?} → ({}, {})",
            state.cursor_monitor_idx,
            state.focus_monitor_idx,
            state.active_monitor_idx,
            monitor.x,
            monitor.y,
        );

        Some((
            ActiveMonitor::from_bounds(monitor),
            state.active_monitor_idx,
        ))
    }

    pub fn all_monitors(&self) -> Vec<ActiveMonitor> {
        let Some(state) = self.client.get_state() else {
            return Vec::new();
        };
        state
            .monitors
            .iter()
            .copied()
            .map(ActiveMonitor::from_bounds)
            .collect()
    }
}
