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
        eprintln!(
            "[launcher/monitor] centered_bounds: monitor=({}, {}, {}x{}) win_size=({}, {}) → x={}, y={}",
            self.inner.x, self.inner.y, self.inner.width, self.inner.height,
            win_size.width.to_f64(), win_size.height.to_f64(),
            x.to_f64(), y.to_f64(),
        );
        Bounds::new(point(x, y), win_size)
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

    pub fn snapshot(&self) -> Option<ActiveMonitor> {
        let state = self.client.get_state()?;

        if state.monitors.is_empty() {
            eprintln!("[launcher/monitor] snapshot: no monitors reported");
            return None;
        }
        if state.monitors.len() == 1 {
            let m = state.monitors[0];
            eprintln!(
                "[launcher/monitor] snapshot: single monitor ({}, {}, {}x{})",
                m.x, m.y, m.width, m.height,
            );
            return Some(ActiveMonitor::from_bounds(m));
        }

        let (monitor, strategy) = if let Some(m) = state.active_monitor() {
            (m, "active")
        } else {
            (state.monitors[0], "fallback[0]")
        };

        eprintln!(
            "[launcher/monitor] snapshot: strategy={} cursor_idx={:?} focus_idx={:?} active_idx={:?} → chosen=({}, {}, {}x{}) monitors=[{}]",
            strategy,
            state.cursor_monitor_idx,
            state.focus_monitor_idx,
            state.active_monitor_idx,
            monitor.x, monitor.y, monitor.width, monitor.height,
            state.monitors.iter()
                .enumerate()
                .map(|(i, m)| format!("{}:({},{},{}x{})", i, m.x, m.y, m.width, m.height))
                .collect::<Vec<_>>()
                .join(", "),
        );

        Some(ActiveMonitor::from_bounds(monitor))
    }
}
