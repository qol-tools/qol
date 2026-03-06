use qol_runtime::MonitorBounds;

use crate::runtime::state::{self, InputState, Stamped};

type MonitorStamp = Option<(MonitorBounds, std::time::Instant)>;

pub(super) struct TickSample {
    pub(super) committed: bool,
    pub(super) cursor_monitor: Option<MonitorBounds>,
    pub(super) cursor_moved: bool,
    pub(super) focus_bounds: Option<MonitorBounds>,
    pub(super) focus_changed: bool,
    pub(super) focus_monitor: Option<MonitorBounds>,
    pub(super) now: std::time::Instant,
}

pub(super) fn apply_updates(input: &mut InputState, sample: &TickSample) -> bool {
    let before = snapshot_input(input);
    let cursor_changed = apply_cursor_update(input, sample);
    let focus_changed = apply_focus_update(input, sample);
    log_state_change(input, before, sample);
    sample.cursor_moved || cursor_changed || focus_changed
}

fn apply_cursor_update(input: &mut InputState, sample: &TickSample) -> bool {
    let Some(monitor) = sample.cursor_monitor else {
        return false;
    };

    let before = input.cursor.as_ref().map(|cursor| cursor.monitor);
    input.update_cursor(monitor, sample.now, sample.cursor_moved);
    sample.cursor_moved && before != input.cursor.as_ref().map(|cursor| cursor.monitor)
}

fn apply_focus_update(input: &mut InputState, sample: &TickSample) -> bool {
    let Some(monitor) = sample.focus_monitor else {
        return false;
    };
    if !sample.focus_changed {
        return false;
    }

    let before = input.focus.as_ref().map(|focus| focus.monitor);
    input.update_focus(monitor, sample.now);
    before != input.focus.as_ref().map(|focus| focus.monitor)
}

fn log_state_change(input: &InputState, before: (MonitorStamp, MonitorStamp), sample: &TickSample) {
    let after = snapshot_input(input);
    if before == after {
        return;
    }

    let active = state::pick_active_monitor(input, zero_monitor());
    let focus_bounds = sample.focus_bounds.map(|b| (b.x, b.y, b.width, b.height));
    let cursor = input.cursor.as_ref().map(|c| (c.monitor.x, c.monitor.y));
    let focus = input.focus.as_ref().map(|f| (f.monitor.x, f.monitor.y));

    log::debug!(
        "[runtime/poll] STATE CHANGE committed={} focus_changed={} focus_bounds={:?} cursor=({:?}) focus=({:?}) → active=({}, {})",
        sample.committed,
        sample.focus_changed,
        focus_bounds,
        cursor,
        focus,
        active.x,
        active.y,
    );
}

fn snapshot_input(input: &InputState) -> (MonitorStamp, MonitorStamp) {
    (
        snapshot_stamp(input.cursor.as_ref()),
        snapshot_stamp(input.focus.as_ref()),
    )
}

fn snapshot_stamp(stamp: Option<&Stamped>) -> MonitorStamp {
    stamp.map(|stamp| (stamp.monitor, stamp.at))
}

fn zero_monitor() -> MonitorBounds {
    MonitorBounds {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    }
}
