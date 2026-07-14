use gpui::*;
use qol_runtime::{MonitorBounds, PlatformState, PlatformStateClient};

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

    pub fn snapshot_cursor(&self) -> Option<(ActiveMonitor, Option<Point<Pixels>>)> {
        let state = self.client.get_state()?;
        resolve_cursor_snapshot(&state)
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

fn resolve_cursor_snapshot(
    state: &PlatformState,
) -> Option<(ActiveMonitor, Option<Point<Pixels>>)> {
    let fallback = state.monitors.first().copied()?;
    let monitor = state
        .cursor_monitor()
        .or_else(|| state.active_monitor())
        .or_else(|| state.focus_monitor())
        .unwrap_or(fallback);
    let cursor = state.cursor.map(|cursor| point(px(cursor.x), px(cursor.y)));
    Some((ActiveMonitor::from_bounds(monitor), cursor))
}

#[cfg(test)]
mod tests {
    use super::resolve_cursor_snapshot;
    use qol_runtime::{CursorPos, MonitorBounds, PlatformState};

    fn monitor(x: f32) -> MonitorBounds {
        MonitorBounds {
            x,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        }
    }

    #[test]
    fn cursor_snapshot_prefers_cursor_monitor_and_preserves_position() {
        let state = PlatformState {
            cursor: Some(CursorPos {
                x: 2500.0,
                y: 700.0,
            }),
            monitors: vec![monitor(0.0), monitor(1920.0)],
            cursor_monitor_idx: Some(1),
            focus_monitor_idx: Some(0),
            active_monitor_idx: Some(0),
            focused_window: None,
        };

        let (monitor, cursor) = resolve_cursor_snapshot(&state).unwrap();
        let cursor = cursor.unwrap();

        assert_eq!(monitor.bounds().origin.x.to_f64(), 1920.0);
        assert_eq!(cursor.x.to_f64(), 2500.0);
        assert_eq!(cursor.y.to_f64(), 700.0);
    }

    #[test]
    fn cursor_snapshot_falls_back_and_rejects_empty_monitor_state() {
        let cases = [(vec![monitor(-1920.0)], Some(-1920.0)), (Vec::new(), None)];

        for (monitors, expected_x) in cases {
            let state = PlatformState {
                cursor: None,
                monitors,
                cursor_monitor_idx: None,
                focus_monitor_idx: None,
                active_monitor_idx: None,
                focused_window: None,
            };
            let resolved = resolve_cursor_snapshot(&state)
                .map(|(monitor, _)| monitor.bounds().origin.x.to_f64());
            assert_eq!(resolved, expected_x);
        }
    }
}
