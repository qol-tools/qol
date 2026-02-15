#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

use gpui::*;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const SETTLE_MS: u64 = 400;

#[derive(Clone, Debug)]
pub struct ActiveMonitor {
    bounds: Bounds<Pixels>,
}

impl ActiveMonitor {
    pub fn centered_bounds(&self, win_size: Size<Pixels>) -> Bounds<Pixels> {
        let x = self.bounds.origin.x + (self.bounds.size.width - win_size.width) / 2.0;
        let y = self.bounds.origin.y + (self.bounds.size.height - win_size.height) / 3.0;
        Bounds::new(point(x, y), win_size)
    }

    pub fn bounds(&self) -> &Bounds<Pixels> {
        &self.bounds
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Stamped {
    pub monitor: ActiveMonitor,
    pub at: Instant,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InputState {
    pub focus: Option<Stamped>,
    pub cursor: Option<Stamped>,
    pub pending_cursor: Option<Stamped>,
}

pub(crate) fn monitor_for_point(monitors: &[Bounds<Pixels>], x: f32, y: f32) -> Option<Bounds<Pixels>> {
    monitors
        .iter()
        .find(|m| {
            let right = m.origin.x + m.size.width;
            let bottom = m.origin.y + m.size.height;
            px(x) >= m.origin.x && px(x) < right && px(y) >= m.origin.y && px(y) < bottom
        })
        .copied()
}

pub(crate) fn track_cursor_monitor(state: &mut InputState, monitor: Bounds<Pixels>, at: Instant) {
    let same_settled = state
        .cursor
        .as_ref()
        .is_some_and(|c| c.monitor.bounds == monitor);
    if same_settled {
        state.pending_cursor = None;
        return;
    }

    let same_pending = state
        .pending_cursor
        .as_ref()
        .is_some_and(|p| p.monitor.bounds == monitor);
    if same_pending {
        return;
    }

    state.pending_cursor = Some(Stamped {
        monitor: ActiveMonitor { bounds: monitor },
        at,
    });
}

pub(crate) fn promote_pending_cursor(state: &mut InputState, now: Instant) {
    if let Some(ref pending) = state.pending_cursor {
        if now.duration_since(pending.at).as_millis() >= SETTLE_MS as u128 {
            state.cursor = state.pending_cursor.take();
        }
    }
}

fn pick_active_monitor(state: &InputState, fallback: Bounds<Pixels>) -> ActiveMonitor {
    match (state.cursor.as_ref(), state.focus.as_ref()) {
        (Some(cursor), Some(focus)) => {
            if cursor.at >= focus.at {
                cursor.monitor.clone()
            } else {
                focus.monitor.clone()
            }
        }
        (Some(cursor), None) => cursor.monitor.clone(),
        (None, Some(focus)) => focus.monitor.clone(),
        (None, None) => ActiveMonitor { bounds: fallback },
    }
}

#[derive(Clone)]
pub struct FocusCache {
    state: Arc<Mutex<InputState>>,
    monitors: Vec<Bounds<Pixels>>,
}

impl FocusCache {
    pub fn start(cx: &App) -> Self {
        let monitors = physical_monitors(cx);
        let state = Arc::new(Mutex::new(InputState::default()));

        #[cfg(target_os = "linux")]
        linux::start_focus_tracking(state.clone(), monitors.clone());

        #[cfg(target_os = "macos")]
        macos::start_focus_tracking(state.clone(), monitors.clone());

        Self { state, monitors }
    }

    pub fn snapshot(&self) -> Option<ActiveMonitor> {
        if self.monitors.is_empty() {
            return None;
        }
        if self.monitors.len() == 1 {
            return Some(ActiveMonitor {
                bounds: self.monitors[0],
            });
        }

        let mut guard = self.state.lock().ok()?;
        let now = Instant::now();
        promote_pending_cursor(&mut guard, now);

        let result = pick_active_monitor(&guard, self.monitors[0]);

        #[cfg(debug_assertions)]
        eprintln!(
            "[monitor] snapshot: cursor={:?}, pending={:?}, focus={:?}",
            guard.cursor.as_ref().map(|c| &c.monitor.bounds),
            guard.pending_cursor.as_ref().map(|p| &p.monitor.bounds),
            guard.focus.as_ref().map(|f| &f.monitor.bounds),
        );

        Some(result)
    }
}

fn physical_monitors(cx: &App) -> Vec<Bounds<Pixels>> {
    #[cfg(target_os = "macos")]
    {
        let cg = macos::display_bounds();
        if cg.len() > 1 {
            return cg;
        }
    }

    let gpui_displays = cx.displays();
    if gpui_displays.len() > 1 {
        return gpui_displays.iter().map(|d| d.bounds()).collect();
    }

    #[cfg(target_os = "linux")]
    {
        let xrandr = linux::xrandr_monitors();
        if xrandr.len() > 1 {
            return xrandr;
        }
    }

    gpui_displays.iter().map(|d| d.bounds()).collect()
}

pub(crate) fn monitor_for_bounds(
    monitors: &[Bounds<Pixels>],
    window: &Bounds<Pixels>,
) -> Option<Bounds<Pixels>> {
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

fn intersection_area(a: &Bounds<Pixels>, b: &Bounds<Pixels>) -> f64 {
    let inter = a.intersect(b);
    if inter.size.width <= px(0.) || inter.size.height <= px(0.) {
        return 0.0;
    }
    inter.size.width.to_f64() * inter.size.height.to_f64()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn mon(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    fn stamped(bounds: Bounds<Pixels>, at: Instant) -> Stamped {
        Stamped {
            monitor: ActiveMonitor { bounds },
            at,
        }
    }

    #[::std::prelude::v1::test]
    fn monitor_for_point_finds_correct_monitor() {
        let monitors = vec![
            mon(0.0, 0.0, 1920.0, 1080.0),
            mon(1920.0, 0.0, 2560.0, 1440.0),
        ];
        let cases = [
            (100.0, 100.0, Some(monitors[0])),
            (1920.0, 500.0, Some(monitors[1])),
            (3000.0, 700.0, Some(monitors[1])),
            (5000.0, 0.0, None),
            (0.0, 1080.0, None),
        ];
        for (x, y, expected) in cases {
            assert_eq!(
                monitor_for_point(&monitors, x, y),
                expected,
                "point: ({x}, {y})"
            );
        }
    }

    #[::std::prelude::v1::test]
    fn monitor_for_point_at_origin() {
        let monitors = vec![
            mon(0.0, 0.0, 1920.0, 1080.0),
            mon(1920.0, 0.0, 2560.0, 1440.0),
        ];
        assert_eq!(monitor_for_point(&monitors, 0.0, 0.0), Some(monitors[0]));
        assert_eq!(monitor_for_point(&monitors, 1920.0, 0.0), Some(monitors[1]));
    }

    #[::std::prelude::v1::test]
    fn track_cursor_monitor_creates_pending_on_monitor_change() {
        let m_a = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_b = mon(1920.0, 0.0, 2560.0, 1440.0);
        let now = Instant::now();
        let mut state = InputState {
            focus: None,
            cursor: Some(stamped(m_a, now - Duration::from_secs(2))),
            pending_cursor: None,
        };

        track_cursor_monitor(&mut state, m_b, now);

        assert_eq!(
            state.pending_cursor.as_ref().map(|p| p.monitor.bounds),
            Some(m_b)
        );
    }

    #[::std::prelude::v1::test]
    fn snapshot_prefers_newer_cursor_over_focus() {
        let m_focus = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_cursor = mon(1920.0, 0.0, 2560.0, 1440.0);
        let now = Instant::now();
        let state = InputState {
            focus: Some(stamped(m_focus, now - Duration::from_secs(2))),
            cursor: Some(stamped(m_cursor, now - Duration::from_secs(1))),
            pending_cursor: None,
        };

        let result = snapshot_from(&state, &[m_focus, m_cursor]);
        assert_eq!(result.unwrap().bounds, m_cursor);
    }

    #[::std::prelude::v1::test]
    fn snapshot_prefers_newer_focus_over_cursor() {
        let m_focus = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_cursor = mon(1920.0, 0.0, 2560.0, 1440.0);
        let now = Instant::now();
        let state = InputState {
            focus: Some(stamped(m_focus, now)),
            cursor: Some(stamped(m_cursor, now - Duration::from_secs(3))),
            pending_cursor: None,
        };

        let result = snapshot_from(&state, &[m_focus, m_cursor]);
        assert_eq!(result.unwrap().bounds, m_focus);
    }

    #[::std::prelude::v1::test]
    fn snapshot_promotes_settled_pending() {
        let m = mon(1920.0, 0.0, 2560.0, 1440.0);
        let fallback = mon(0.0, 0.0, 1920.0, 1080.0);
        let mut state = InputState {
            focus: None,
            cursor: None,
            pending_cursor: Some(stamped(m, Instant::now() - Duration::from_millis(500))),
        };
        let result = snapshot_from_mut(&mut state, &[fallback, m]);
        assert_eq!(result.unwrap().bounds, m);
        assert!(state.cursor.is_some());
        assert!(state.pending_cursor.is_none());
    }

    #[::std::prelude::v1::test]
    fn snapshot_ignores_unsettled_pending() {
        let m_focus = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_pending = mon(1920.0, 0.0, 2560.0, 1440.0);
        let mut state = InputState {
            focus: Some(stamped(m_focus, Instant::now())),
            cursor: None,
            pending_cursor: Some(stamped(m_pending, Instant::now())),
        };
        let result = snapshot_from_mut(&mut state, &[m_focus, m_pending]);
        assert_eq!(result.unwrap().bounds, m_focus);
    }

    #[::std::prelude::v1::test]
    fn snapshot_returns_fallback_with_no_signals() {
        let fallback = mon(0.0, 0.0, 1920.0, 1080.0);
        let state = InputState::default();
        let result = snapshot_from(&state, &[fallback]);
        assert_eq!(result.unwrap().bounds, fallback);
    }

    fn snapshot_from(state: &InputState, monitors: &[Bounds<Pixels>]) -> Option<ActiveMonitor> {
        let mut state = state.clone();
        snapshot_from_mut(&mut state, monitors)
    }

    fn snapshot_from_mut(
        state: &mut InputState,
        monitors: &[Bounds<Pixels>],
    ) -> Option<ActiveMonitor> {
        let now = Instant::now();
        promote_pending_cursor(state, now);
        Some(pick_active_monitor(state, monitors[0]))
    }
}
