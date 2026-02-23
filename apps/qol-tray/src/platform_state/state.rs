use qol_runtime::MonitorBounds;
use std::time::Instant;

#[derive(Clone, Debug)]
pub(crate) struct Stamped {
    pub monitor: MonitorBounds,
    pub at: Instant,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InputState {
    pub focus: Option<Stamped>,
    pub cursor: Option<Stamped>,
}

impl InputState {
    pub(crate) fn update_cursor(&mut self, monitor: MonitorBounds, at: Instant, committed: bool) {
        if !committed {
            return;
        }
        let same = self.cursor.as_ref().is_some_and(|c| c.monitor == monitor);
        if same {
            return;
        }
        self.cursor = Some(Stamped { monitor, at });
    }

    pub(crate) fn update_focus(&mut self, monitor: MonitorBounds, at: Instant) {
        let same = self.focus.as_ref().is_some_and(|f| f.monitor == monitor);
        if same {
            return;
        }
        self.focus = Some(Stamped { monitor, at });
    }
}

pub(crate) fn monitor_for_point(monitors: &[MonitorBounds], x: f32, y: f32) -> Option<MonitorBounds> {
    monitors.iter().find(|m| {
        let right = m.x + m.width;
        let bottom = m.y + m.height;
        x >= m.x && x < right && y >= m.y && y < bottom
    }).copied()
}

pub(crate) fn pick_active_monitor(state: &InputState, fallback: MonitorBounds) -> MonitorBounds {
    match (state.cursor.as_ref(), state.focus.as_ref()) {
        (Some(cursor), Some(focus)) => {
            if cursor.at >= focus.at {
                cursor.monitor
            } else {
                focus.monitor
            }
        }
        (Some(cursor), None) => cursor.monitor,
        (None, Some(focus)) => focus.monitor,
        (None, None) => fallback,
    }
}

pub(crate) fn monitor_for_bounds(monitors: &[MonitorBounds], window: &MonitorBounds) -> Option<MonitorBounds> {
    monitors
        .iter()
        .filter_map(|m| {
            let area = intersection_area(window, m);
            if area > 0.0 {
                Some((*m, area))
            } else {
                None
            }
        })
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(m, _)| m)
}

fn intersection_area(a: &MonitorBounds, b: &MonitorBounds) -> f64 {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    let w = (right - left).max(0.0) as f64;
    let h = (bottom - top).max(0.0) as f64;
    w * h
}
