use gpui::*;
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct ActiveMonitor {
    bounds: Bounds<Pixels>,
}

impl ActiveMonitor {
    pub(crate) fn new(bounds: Bounds<Pixels>) -> Self {
        Self { bounds }
    }

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
}

impl InputState {
    pub(crate) fn update_cursor(&mut self, monitor: Bounds<Pixels>, at: Instant, committed: bool) {
        if !committed {
            return;
        }
        let same = self
            .cursor
            .as_ref()
            .is_some_and(|c| c.monitor.bounds == monitor);
        if same {
            return;
        }
        self.cursor = Some(Stamped {
            monitor: ActiveMonitor::new(monitor),
            at,
        });
    }

    pub(crate) fn update_focus(&mut self, monitor: Bounds<Pixels>, at: Instant) {
        let same = self.focus.as_ref().is_some_and(|f| f.monitor.bounds == monitor);
        if same {
            return;
        }
        self.focus = Some(Stamped {
            monitor: ActiveMonitor::new(monitor),
            at,
        });
    }
}

pub(crate) fn monitor_for_point(
    monitors: &[Bounds<Pixels>],
    x: f32,
    y: f32,
) -> Option<Bounds<Pixels>> {
    monitors
        .iter()
        .find(|m| {
            let right = m.origin.x + m.size.width;
            let bottom = m.origin.y + m.size.height;
            px(x) >= m.origin.x && px(x) < right && px(y) >= m.origin.y && px(y) < bottom
        })
        .copied()
}

pub(crate) fn pick_active_monitor(state: &InputState, fallback: Bounds<Pixels>) -> ActiveMonitor {
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
        (None, None) => ActiveMonitor::new(fallback),
    }
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
    fn pick_prefers_newer_cursor_over_focus() {
        let m_focus = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_cursor = mon(1920.0, 0.0, 2560.0, 1440.0);
        let now = Instant::now();
        let state = InputState {
            focus: Some(stamped(m_focus, now - Duration::from_secs(2))),
            cursor: Some(stamped(m_cursor, now - Duration::from_secs(1))),
        };
        let result = pick_active_monitor(&state, m_focus);
        assert_eq!(result.bounds, m_cursor);
    }

    #[::std::prelude::v1::test]
    fn pick_prefers_newer_focus_over_cursor() {
        let m_focus = mon(0.0, 0.0, 1920.0, 1080.0);
        let m_cursor = mon(1920.0, 0.0, 2560.0, 1440.0);
        let now = Instant::now();
        let state = InputState {
            focus: Some(stamped(m_focus, now)),
            cursor: Some(stamped(m_cursor, now - Duration::from_secs(3))),
        };
        let result = pick_active_monitor(&state, m_focus);
        assert_eq!(result.bounds, m_focus);
    }

    #[::std::prelude::v1::test]
    fn pick_returns_fallback_with_no_signals() {
        let fallback = mon(0.0, 0.0, 1920.0, 1080.0);
        let state = InputState::default();
        let result = pick_active_monitor(&state, fallback);
        assert_eq!(result.bounds, fallback);
    }

    #[::std::prelude::v1::test]
    fn update_cursor_only_commits_when_flagged() {
        let m = mon(0.0, 0.0, 1920.0, 1080.0);
        let mut state = InputState::default();

        state.update_cursor(m, Instant::now(), false);
        assert!(state.cursor.is_none(), "should not commit when uncommitted");

        state.update_cursor(m, Instant::now(), true);
        assert_eq!(state.cursor.as_ref().unwrap().monitor.bounds, m);
    }

    #[::std::prelude::v1::test]
    fn update_cursor_skips_same_monitor() {
        let m = mon(0.0, 0.0, 1920.0, 1080.0);
        let mut state = InputState::default();
        let t1 = Instant::now();
        state.update_cursor(m, t1, true);
        let t2 = Instant::now();
        state.update_cursor(m, t2, true);
        assert_eq!(
            state.cursor.as_ref().unwrap().at,
            t1,
            "should not update timestamp for same monitor"
        );
    }

    #[::std::prelude::v1::test]
    fn update_focus_commits_new_monitor() {
        let m = mon(0.0, 0.0, 1920.0, 1080.0);
        let mut state = InputState::default();
        state.update_focus(m,Instant::now());
        assert_eq!(state.focus.as_ref().unwrap().monitor.bounds, m);
    }

    #[::std::prelude::v1::test]
    fn update_focus_skips_same_monitor() {
        let m = mon(0.0, 0.0, 1920.0, 1080.0);
        let mut state = InputState::default();
        let t1 = Instant::now();
        state.update_focus(m,t1);
        let t2 = Instant::now();
        state.update_focus(m,t2);
        assert_eq!(
            state.focus.as_ref().unwrap().at,
            t1,
            "should not update timestamp for same monitor"
        );
    }
}
