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
    let focused_window = focused_window(shared);

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
        focused_window,
    }
}

fn cursor_monitor_idx(input: &InputState, monitors: &[MonitorBounds]) -> Option<usize> {
    let Some(cursor) = input.cursor.as_ref() else {
        return None;
    };
    monitor_idx(monitors, cursor.monitor)
}

fn focus_monitor_idx(input: &InputState, monitors: &[MonitorBounds]) -> Option<usize> {
    let Some(focus) = input.focus.as_ref() else {
        return None;
    };
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
