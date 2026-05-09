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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn mon(x: f32) -> MonitorBounds {
        MonitorBounds {
            x,
            y: 0.0,
            width: 1000.0,
            height: 1000.0,
        }
    }

    fn empty_sample(now: Instant) -> TickSample {
        TickSample {
            committed: false,
            cursor_monitor: None,
            cursor_moved: false,
            focus_bounds: None,
            focus_changed: false,
            focus_monitor: None,
            now,
        }
    }

    #[test]
    fn apply_updates_returns_false_when_nothing_to_apply() {
        let mut input = InputState::default();
        let sample = empty_sample(Instant::now());
        assert!(!apply_updates(&mut input, &sample));
        assert!(input.cursor.is_none());
        assert!(input.focus.is_none());
    }

    #[test]
    fn apply_updates_returns_true_when_cursor_actually_moved_to_new_monitor() {
        let mut input = InputState::default();
        let m = mon(0.0);
        let sample = TickSample {
            cursor_monitor: Some(m),
            cursor_moved: true,
            ..empty_sample(Instant::now())
        };
        assert!(apply_updates(&mut input, &sample));
        assert_eq!(input.cursor.as_ref().map(|c| c.monitor), Some(m));
    }

    #[test]
    fn apply_updates_returns_true_when_focus_changed_flag_and_monitor_set() {
        let mut input = InputState::default();
        let m = mon(100.0);
        let sample = TickSample {
            focus_monitor: Some(m),
            focus_changed: true,
            ..empty_sample(Instant::now())
        };
        assert!(apply_updates(&mut input, &sample));
        assert_eq!(input.focus.as_ref().map(|f| f.monitor), Some(m));
    }

    #[test]
    fn apply_updates_skips_focus_update_when_focus_changed_flag_is_false() {
        let mut input = InputState::default();
        let m = mon(100.0);
        let sample = TickSample {
            focus_monitor: Some(m),
            focus_changed: false,
            ..empty_sample(Instant::now())
        };
        assert!(
            !apply_updates(&mut input, &sample),
            "focus_monitor without focus_changed must not stamp",
        );
        assert!(input.focus.is_none(), "focus must remain unset");
    }

    #[test]
    fn apply_updates_returns_true_for_cursor_moved_even_when_state_does_not_observably_change() {
        // moved=true with same monitor that's already stamped: state may not change,
        // but the function returns true because cursor_moved itself is "activity".
        let mut input = InputState::default();
        let m = mon(0.0);
        input.cursor = Some(crate::runtime::state::Stamped {
            monitor: m,
            at: Instant::now(),
        });
        let sample = TickSample {
            cursor_monitor: Some(m),
            cursor_moved: true,
            ..empty_sample(Instant::now())
        };
        assert!(
            apply_updates(&mut input, &sample),
            "cursor_moved alone signals activity even when stamp is unchanged",
        );
    }
}
