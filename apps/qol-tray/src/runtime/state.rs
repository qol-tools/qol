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
    pub(crate) fn update_cursor(&mut self, monitor: MonitorBounds, at: Instant, moved: bool) {
        if !moved {
            return;
        }
        let same_monitor = self.cursor.as_ref().is_some_and(|c| c.monitor == monitor);
        // Re-stamp if: (a) cursor changed monitors, or
        // (b) cursor moved and focus currently has a newer timestamp (reclaim from focus)
        let focus_is_newer = self.focus.as_ref().is_some_and(|f|
            self.cursor.as_ref().map_or(true, |c| f.at > c.at)
        );
        if !same_monitor || focus_is_newer {
            log::debug!("[runtime/state] cursor STAMPED mon=({}, {}) at={:?} reason={}",
                monitor.x, monitor.y, at,
                if !same_monitor { "monitor_change" } else { "reclaim_from_focus" });
            self.cursor = Some(Stamped { monitor, at });
        }
    }

    pub(crate) fn update_focus(&mut self, monitor: MonitorBounds, at: Instant) {
        log::debug!("[runtime/state] focus STAMPED mon=({}, {}) at={:?}",
            monitor.x, monitor.y, at);
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
    let result = match (state.cursor.as_ref(), state.focus.as_ref()) {
        (Some(cursor), Some(focus)) => {
            let winner = if cursor.at > focus.at { "cursor" } else { "focus" };
            log::debug!("[runtime/pick] cursor_mon=({},{}) cursor_at={:?} focus_mon=({},{}) focus_at={:?} → {}",
                cursor.monitor.x, cursor.monitor.y, cursor.at,
                focus.monitor.x, focus.monitor.y, focus.at,
                winner);
            if cursor.at > focus.at {
                cursor.monitor
            } else {
                focus.monitor
            }
        }
        (Some(cursor), None) => {
            log::debug!("[runtime/pick] cursor only → ({}, {})", cursor.monitor.x, cursor.monitor.y);
            cursor.monitor
        }
        (None, Some(focus)) => {
            log::debug!("[runtime/pick] focus only → ({}, {})", focus.monitor.x, focus.monitor.y);
            focus.monitor
        }
        (None, None) => {
            log::debug!("[runtime/pick] fallback");
            fallback
        }
    };
    result
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
