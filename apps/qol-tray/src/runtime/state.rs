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
        let same_monitor = self.cursor.as_ref().is_some_and(|c| c.monitor == monitor);
        let focus_is_newer = self
            .focus
            .as_ref()
            .is_some_and(|f| self.cursor.as_ref().is_none_or(|c| f.at > c.at));
        let focus_elsewhere = self.focus.as_ref().is_some_and(|f| f.monitor != monitor);
        if !moved {
            if same_monitor && focus_is_newer && focus_elsewhere {
                log::debug!(
                    "[runtime/state] cursor STAMPED mon=({}, {}) at={:?} reason=still_here_reclaim",
                    monitor.x,
                    monitor.y,
                    at,
                );
                self.cursor = Some(Stamped { monitor, at });
            }
            return;
        }
        if !same_monitor || focus_is_newer {
            log::debug!(
                "[runtime/state] cursor STAMPED mon=({}, {}) at={:?} reason={}",
                monitor.x,
                monitor.y,
                at,
                cursor_stamp_reason(same_monitor)
            );
            self.cursor = Some(Stamped { monitor, at });
        }
    }

    pub(crate) fn update_focus(&mut self, monitor: MonitorBounds, at: Instant) {
        log::debug!(
            "[runtime/state] focus STAMPED mon=({}, {}) at={:?}",
            monitor.x,
            monitor.y,
            at
        );
        self.focus = Some(Stamped { monitor, at });
    }
}

fn cursor_stamp_reason(same_monitor: bool) -> &'static str {
    if same_monitor {
        "reclaim_from_focus"
    } else {
        "monitor_change"
    }
}

pub(crate) fn monitor_for_point(
    monitors: &[MonitorBounds],
    x: f32,
    y: f32,
) -> Option<MonitorBounds> {
    monitors
        .iter()
        .find(|m| {
            let right = m.x + m.width;
            let bottom = m.y + m.height;
            x >= m.x && x < right && y >= m.y && y < bottom
        })
        .copied()
}

pub(crate) fn pick_active_monitor(state: &InputState, fallback: MonitorBounds) -> MonitorBounds {
    match (state.cursor.as_ref(), state.focus.as_ref()) {
        (Some(cursor), Some(focus)) => {
            log_pick_both(cursor, focus);
            if cursor.at > focus.at {
                cursor.monitor
            } else {
                focus.monitor
            }
        }
        (Some(cursor), None) => {
            log::debug!(
                "[runtime/pick] cursor only → ({}, {})",
                cursor.monitor.x,
                cursor.monitor.y
            );
            cursor.monitor
        }
        (None, Some(focus)) => {
            log::debug!(
                "[runtime/pick] focus only → ({}, {})",
                focus.monitor.x,
                focus.monitor.y
            );
            focus.monitor
        }
        (None, None) => {
            log::debug!("[runtime/pick] fallback");
            fallback
        }
    }
}

fn log_pick_both(cursor: &Stamped, focus: &Stamped) {
    let winner = if cursor.at > focus.at {
        "cursor"
    } else {
        "focus"
    };
    log::debug!(
        "[runtime/pick] cursor_mon=({},{}) cursor_at={:?} focus_mon=({},{}) focus_at={:?} → {}",
        cursor.monitor.x,
        cursor.monitor.y,
        cursor.at,
        focus.monitor.x,
        focus.monitor.y,
        focus.at,
        winner
    );
}

pub(crate) fn monitor_for_bounds(
    monitors: &[MonitorBounds],
    window: &MonitorBounds,
) -> Option<MonitorBounds> {
    monitors
        .iter()
        .filter_map(|m| {
            let area = intersection_area(window, m);
            if area <= 0.0 {
                return None;
            }
            Some((*m, area))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn mon(x: f32, y: f32, w: f32, h: f32) -> MonitorBounds {
        MonitorBounds {
            x,
            y,
            width: w,
            height: h,
        }
    }

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    #[test]
    fn monitor_for_point_returns_the_containing_monitor() {
        let monitors = vec![
            mon(0.0, 0.0, 1920.0, 1080.0),
            mon(1920.0, 0.0, 1280.0, 720.0),
        ];
        assert_eq!(monitor_for_point(&monitors, 100.0, 50.0), Some(monitors[0]));
        assert_eq!(
            monitor_for_point(&monitors, 2000.0, 50.0),
            Some(monitors[1])
        );
    }

    #[test]
    fn monitor_for_point_treats_left_top_inclusive_right_bottom_exclusive() {
        let m = mon(100.0, 200.0, 50.0, 25.0);
        let monitors = vec![m];
        // Inclusive top-left:
        assert_eq!(monitor_for_point(&monitors, 100.0, 200.0), Some(m));
        // Just inside the lower-right:
        assert_eq!(monitor_for_point(&monitors, 149.9, 224.9), Some(m));
        // Exclusive right edge:
        assert_eq!(monitor_for_point(&monitors, 150.0, 200.0), None);
        // Exclusive bottom edge:
        assert_eq!(monitor_for_point(&monitors, 100.0, 225.0), None);
    }

    #[test]
    fn monitor_for_point_returns_none_when_outside_all_monitors() {
        let monitors = vec![mon(0.0, 0.0, 100.0, 100.0)];
        let cases = [(-1.0, 50.0), (50.0, -1.0), (200.0, 50.0), (50.0, 200.0)];
        for (x, y) in cases {
            assert!(
                monitor_for_point(&monitors, x, y).is_none(),
                "point ({x},{y}) should be outside",
            );
        }
        assert_eq!(monitor_for_point(&[], 0.0, 0.0), None);
    }

    #[test]
    fn monitor_for_point_picks_first_match_when_monitors_overlap() {
        let a = mon(0.0, 0.0, 100.0, 100.0);
        let b = mon(50.0, 50.0, 100.0, 100.0);
        let monitors = vec![a, b];
        // Point (60, 60) is inside both; first match wins.
        assert_eq!(monitor_for_point(&monitors, 60.0, 60.0), Some(a));
    }

    #[test]
    fn pick_active_monitor_returns_fallback_when_state_is_empty() {
        let fb = mon(0.0, 0.0, 1920.0, 1080.0);
        assert_eq!(pick_active_monitor(&InputState::default(), fb), fb);
    }

    #[test]
    fn pick_active_monitor_uses_cursor_when_only_cursor_set() {
        let m = mon(100.0, 0.0, 800.0, 600.0);
        let state = InputState {
            cursor: Some(Stamped {
                monitor: m,
                at: Instant::now(),
            }),
            ..InputState::default()
        };
        assert_eq!(pick_active_monitor(&state, mon(0.0, 0.0, 1.0, 1.0)), m);
    }

    #[test]
    fn pick_active_monitor_uses_focus_when_only_focus_set() {
        let m = mon(200.0, 0.0, 800.0, 600.0);
        let state = InputState {
            focus: Some(Stamped {
                monitor: m,
                at: Instant::now(),
            }),
            ..InputState::default()
        };
        assert_eq!(pick_active_monitor(&state, mon(0.0, 0.0, 1.0, 1.0)), m);
    }

    #[test]
    fn pick_active_monitor_picks_the_more_recent_when_both_set() {
        let base = Instant::now();
        let cursor_mon = mon(100.0, 0.0, 800.0, 600.0);
        let focus_mon = mon(900.0, 0.0, 800.0, 600.0);
        let mut state = InputState {
            cursor: Some(Stamped {
                monitor: cursor_mon,
                at: at(base, 100),
            }),
            focus: Some(Stamped {
                monitor: focus_mon,
                at: at(base, 50),
            }),
        };
        assert_eq!(
            pick_active_monitor(&state, mon(0.0, 0.0, 1.0, 1.0)),
            cursor_mon,
            "cursor is newer, cursor wins",
        );
        state.focus = Some(Stamped {
            monitor: focus_mon,
            at: at(base, 200),
        });
        assert_eq!(
            pick_active_monitor(&state, mon(0.0, 0.0, 1.0, 1.0)),
            focus_mon,
            "focus is newer, focus wins",
        );
    }

    #[test]
    fn pick_active_monitor_breaks_equal_timestamp_tie_in_favor_of_focus() {
        let now = Instant::now();
        let cursor_mon = mon(100.0, 0.0, 800.0, 600.0);
        let focus_mon = mon(900.0, 0.0, 800.0, 600.0);
        let state = InputState {
            cursor: Some(Stamped {
                monitor: cursor_mon,
                at: now,
            }),
            focus: Some(Stamped {
                monitor: focus_mon,
                at: now,
            }),
        };
        // cursor.at > focus.at is false when equal; falls through to focus.
        assert_eq!(
            pick_active_monitor(&state, mon(0.0, 0.0, 1.0, 1.0)),
            focus_mon,
        );
    }

    #[test]
    fn monitor_for_bounds_picks_the_monitor_with_max_overlap_area() {
        let left = mon(0.0, 0.0, 1000.0, 1000.0);
        let right = mon(1000.0, 0.0, 1000.0, 1000.0);
        let monitors = vec![left, right];
        // Window straddling the seam, 80% on the right:
        let window = mon(800.0, 0.0, 1000.0, 1000.0);
        assert_eq!(monitor_for_bounds(&monitors, &window), Some(right));
        // Window fully on the left:
        assert_eq!(
            monitor_for_bounds(&monitors, &mon(10.0, 10.0, 100.0, 100.0)),
            Some(left),
        );
    }

    #[test]
    fn monitor_for_bounds_returns_none_when_window_is_fully_outside() {
        let monitors = vec![mon(0.0, 0.0, 100.0, 100.0)];
        assert!(monitor_for_bounds(&monitors, &mon(200.0, 200.0, 50.0, 50.0)).is_none());
        assert!(monitor_for_bounds(&[], &mon(0.0, 0.0, 1.0, 1.0)).is_none());
    }

    #[test]
    fn monitor_for_bounds_treats_zero_area_overlap_as_no_match() {
        // Window touching the right edge but with zero overlap (right = left of the next monitor).
        let monitors = vec![mon(0.0, 0.0, 100.0, 100.0), mon(100.0, 0.0, 100.0, 100.0)];
        let window = mon(100.0, 0.0, 0.0, 100.0); // zero-width window at the seam
        assert_eq!(monitor_for_bounds(&monitors, &window), None);
    }

    #[test]
    fn update_focus_replaces_focus_stamp() {
        let base = Instant::now();
        let mut state = InputState::default();
        let m1 = mon(0.0, 0.0, 100.0, 100.0);
        let m2 = mon(100.0, 0.0, 100.0, 100.0);
        state.update_focus(m1, base);
        assert_eq!(state.focus.as_ref().map(|s| s.monitor), Some(m1));
        state.update_focus(m2, at(base, 10));
        assert_eq!(state.focus.as_ref().map(|s| s.monitor), Some(m2));
        assert_eq!(state.focus.as_ref().map(|s| s.at), Some(at(base, 10)));
    }

    #[test]
    fn update_cursor_stamps_when_moved_to_new_monitor() {
        let base = Instant::now();
        let mut state = InputState::default();
        let m1 = mon(0.0, 0.0, 100.0, 100.0);
        let m2 = mon(100.0, 0.0, 100.0, 100.0);
        state.update_cursor(m1, base, true);
        assert_eq!(state.cursor.as_ref().map(|s| s.monitor), Some(m1));
        state.update_cursor(m2, at(base, 10), true);
        assert_eq!(state.cursor.as_ref().map(|s| s.monitor), Some(m2));
    }

    #[test]
    fn update_cursor_no_op_when_not_moved_and_no_reclaim_signal() {
        let base = Instant::now();
        let mut state = InputState::default();
        let m = mon(0.0, 0.0, 100.0, 100.0);
        state.update_cursor(m, base, false);
        assert!(
            state.cursor.is_none(),
            "no cursor stamp when not moved and nothing to reclaim",
        );
    }

    #[test]
    fn update_cursor_reclaims_when_not_moved_but_focus_is_newer_and_elsewhere() {
        let base = Instant::now();
        let mut state = InputState::default();
        let here = mon(0.0, 0.0, 100.0, 100.0);
        let elsewhere = mon(100.0, 0.0, 100.0, 100.0);
        // Initial cursor stamp on `here`, older than the focus stamp on `elsewhere`.
        state.cursor = Some(Stamped {
            monitor: here,
            at: at(base, 0),
        });
        state.focus = Some(Stamped {
            monitor: elsewhere,
            at: at(base, 100),
        });
        // Cursor poll says we are still on `here`, no movement.
        state.update_cursor(here, at(base, 200), false);
        assert_eq!(
            state.cursor.as_ref().map(|s| s.at),
            Some(at(base, 200)),
            "stale cursor must be re-stamped to reclaim activity from focus",
        );
    }

    #[test]
    fn update_cursor_does_not_reclaim_when_focus_is_on_same_monitor() {
        let base = Instant::now();
        let mut state = InputState::default();
        let here = mon(0.0, 0.0, 100.0, 100.0);
        state.cursor = Some(Stamped {
            monitor: here,
            at: at(base, 0),
        });
        state.focus = Some(Stamped {
            monitor: here,
            at: at(base, 100),
        });
        state.update_cursor(here, at(base, 200), false);
        assert_eq!(
            state.cursor.as_ref().map(|s| s.at),
            Some(at(base, 0)),
            "focus on same monitor: no reclaim needed, stamp unchanged",
        );
    }

    #[test]
    fn update_cursor_keeps_existing_when_moved_but_same_monitor_and_no_newer_focus() {
        let base = Instant::now();
        let mut state = InputState::default();
        let here = mon(0.0, 0.0, 100.0, 100.0);
        state.cursor = Some(Stamped {
            monitor: here,
            at: at(base, 100),
        });
        state.update_cursor(here, at(base, 200), true);
        // Movement on the same monitor without a newer focus elsewhere doesn't refresh the
        // timestamp - the cursor is the source of truth and was already on this monitor.
        assert_eq!(
            state.cursor.as_ref().map(|s| s.at),
            Some(at(base, 100)),
            "moved within the same monitor with stale focus: stamp kept",
        );
    }

    #[test]
    fn update_cursor_refreshes_when_moved_within_same_monitor_but_focus_is_newer() {
        let base = Instant::now();
        let mut state = InputState::default();
        let here = mon(0.0, 0.0, 100.0, 100.0);
        state.cursor = Some(Stamped {
            monitor: here,
            at: at(base, 0),
        });
        state.focus = Some(Stamped {
            monitor: here,
            at: at(base, 100),
        });
        state.update_cursor(here, at(base, 200), true);
        assert_eq!(
            state.cursor.as_ref().map(|s| s.at),
            Some(at(base, 200)),
            "moved within same monitor with newer focus: stamp refreshed so cursor wins again",
        );
    }
}
