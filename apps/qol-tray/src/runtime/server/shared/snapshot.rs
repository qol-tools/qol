use qol_runtime::{CursorPos, MonitorBounds, PlatformState, WindowBounds};

use super::SharedState;
use crate::runtime::state::{self, InputState};

pub(super) fn build_state(shared: &SharedState) -> PlatformState {
    let monitors = shared.monitors();
    let cursor = shared.cursor_pos().map(|(x, y)| CursorPos { x, y });
    let input = shared.input();
    let cursor_monitor_idx = cursor_monitor_idx(&input, &monitors);
    let focus_monitor_idx = focus_monitor_idx(&input, &monitors);
    let active_monitor_idx = active_monitor_idx(&input, &monitors);
    log::debug!(
        "[runtime/build_state] GET_STATE cursor_idx={:?} focus_idx={:?} active_idx={:?}",
        cursor_monitor_idx,
        focus_monitor_idx,
        active_monitor_idx
    );
    PlatformState {
        cursor,
        monitors,
        cursor_monitor_idx,
        focus_monitor_idx,
        active_monitor_idx,
        focused_window: focused_window(shared),
    }
}

fn cursor_monitor_idx(input: &InputState, monitors: &[MonitorBounds]) -> Option<usize> {
    let cursor = input.cursor.as_ref()?;
    monitor_idx(monitors, cursor.monitor)
}

fn focus_monitor_idx(input: &InputState, monitors: &[MonitorBounds]) -> Option<usize> {
    let focus = input.focus.as_ref()?;
    monitor_idx(monitors, focus.monitor)
}

fn active_monitor_idx(input: &InputState, monitors: &[MonitorBounds]) -> Option<usize> {
    let active = state::pick_active_monitor(input, fallback_monitor(monitors));
    monitor_idx(monitors, active)
}

fn monitor_idx(monitors: &[MonitorBounds], monitor: MonitorBounds) -> Option<usize> {
    monitors.iter().position(|candidate| *candidate == monitor)
}

fn focused_window(shared: &SharedState) -> Option<WindowBounds> {
    shared.focused_window().map(|bounds| WindowBounds {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: bounds.height,
    })
}

fn fallback_monitor(monitors: &[MonitorBounds]) -> MonitorBounds {
    monitors.first().copied().unwrap_or(MonitorBounds {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::state::Stamped;
    use std::time::Instant;

    fn mon(x: f32) -> MonitorBounds {
        MonitorBounds {
            x,
            y: 0.0,
            width: 1000.0,
            height: 1000.0,
        }
    }

    #[test]
    fn build_state_returns_none_indices_when_inputs_are_empty() {
        let shared = SharedState::new(vec![mon(0.0), mon(2000.0)]);
        let state = build_state(&shared);
        assert!(state.cursor.is_none());
        assert!(state.cursor_monitor_idx.is_none());
        assert!(state.focus_monitor_idx.is_none());
        assert_eq!(
            state.active_monitor_idx,
            Some(0),
            "active falls back to first monitor when nothing else is set",
        );
        assert_eq!(state.monitors.len(), 2);
    }

    #[test]
    fn build_state_resolves_cursor_focus_active_indices_separately() {
        let monitors = vec![mon(0.0), mon(2000.0)];
        let shared = SharedState::new(monitors.clone());
        shared.set_cursor_pos(Some((10.0, 10.0)));
        let now = Instant::now();
        shared.with_input(|input| {
            input.cursor = Some(Stamped {
                monitor: monitors[0],
                at: now,
            });
            input.focus = Some(Stamped {
                monitor: monitors[1],
                at: now + std::time::Duration::from_millis(10),
            });
        });

        let state = build_state(&shared);

        assert_eq!(state.cursor_monitor_idx, Some(0));
        assert_eq!(state.focus_monitor_idx, Some(1));
        assert_eq!(
            state.active_monitor_idx,
            Some(1),
            "newer focus stamp wins active",
        );
        assert_eq!(state.cursor.map(|c| (c.x, c.y)), Some((10.0, 10.0)));
    }

    #[test]
    fn build_state_returns_focused_window_when_set() {
        let shared = SharedState::new(vec![mon(0.0)]);
        let bounds = MonitorBounds {
            x: 5.0,
            y: 6.0,
            width: 7.0,
            height: 8.0,
        };
        shared.store_focused_window(Some(bounds));
        let state = build_state(&shared);
        let win = state.focused_window.expect("focused_window set");
        assert_eq!((win.x, win.y, win.width, win.height), (5.0, 6.0, 7.0, 8.0));
    }

    #[test]
    fn build_state_returns_none_focused_window_when_unset() {
        let shared = SharedState::new(vec![mon(0.0)]);
        let state = build_state(&shared);
        assert!(state.focused_window.is_none());
    }

    #[test]
    fn build_state_returns_none_indices_when_input_monitor_not_in_list() {
        let monitors = vec![mon(0.0)];
        let shared = SharedState::new(monitors);
        let now = Instant::now();
        shared.with_input(|input| {
            input.cursor = Some(Stamped {
                monitor: mon(9999.0),
                at: now,
            });
            input.focus = Some(Stamped {
                monitor: mon(9999.0),
                at: now,
            });
        });
        let state = build_state(&shared);
        assert_eq!(state.cursor_monitor_idx, None);
        assert_eq!(state.focus_monitor_idx, None);
        assert_eq!(
            state.active_monitor_idx, None,
            "active falls through to None when cursor/focus monitors aren't in the list",
        );
    }
}
