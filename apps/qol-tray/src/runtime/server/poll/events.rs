use qol_runtime::protocol::RuntimeEvent;
use qol_runtime::MonitorBounds;

use super::super::shared::SharedState;
use crate::runtime::state::{self, InputState};

pub(super) struct EventTracker {
    prev_active_idx: Option<usize>,
    prev_focus_idx: Option<usize>,
}

impl EventTracker {
    pub(super) fn new() -> Self {
        Self {
            prev_active_idx: None,
            prev_focus_idx: None,
        }
    }

    pub(super) fn build(
        &mut self,
        shared: &SharedState,
        monitors: &[MonitorBounds],
        monitors_changed: bool,
    ) -> Vec<RuntimeEvent> {
        let input = shared.input();
        let current_active_idx = active_monitor_idx(&input, monitors);
        let current_focus_idx = focus_monitor_idx(&input, monitors);
        let mut events = Vec::new();

        if let Some(event) = monitors_changed_event(shared, monitors_changed) {
            events.push(event);
        }
        if let Some(event) = self.active_monitor_event(monitors, current_active_idx) {
            events.push(event);
        }
        if let Some(event) = self.focus_event(monitors, current_focus_idx) {
            events.push(event);
        }

        events
    }

    fn active_monitor_event(
        &mut self,
        monitors: &[MonitorBounds],
        current_idx: Option<usize>,
    ) -> Option<RuntimeEvent> {
        if current_idx == self.prev_active_idx {
            return None;
        }

        self.prev_active_idx = current_idx;
        Some(RuntimeEvent::ActiveMonitorChanged {
            monitor_idx: current_idx,
            monitor: monitor_at(monitors, current_idx),
        })
    }

    fn focus_event(
        &mut self,
        monitors: &[MonitorBounds],
        current_idx: Option<usize>,
    ) -> Option<RuntimeEvent> {
        if current_idx == self.prev_focus_idx {
            return None;
        }

        self.prev_focus_idx = current_idx;
        Some(RuntimeEvent::FocusChanged {
            monitor_idx: current_idx,
            monitor: monitor_at(monitors, current_idx),
        })
    }
}

fn monitors_changed_event(shared: &SharedState, changed: bool) -> Option<RuntimeEvent> {
    if !changed {
        return None;
    }

    Some(RuntimeEvent::MonitorsChanged {
        monitors: shared.monitors(),
    })
}

fn active_monitor_idx(input: &InputState, monitors: &[MonitorBounds]) -> Option<usize> {
    let active = state::pick_active_monitor(input, fallback_monitor(monitors));
    monitor_idx(monitors, active)
}

fn focus_monitor_idx(input: &InputState, monitors: &[MonitorBounds]) -> Option<usize> {
    let Some(focus) = input.focus.as_ref() else {
        return None;
    };
    monitor_idx(monitors, focus.monitor)
}

fn monitor_idx(monitors: &[MonitorBounds], monitor: MonitorBounds) -> Option<usize> {
    monitors.iter().position(|candidate| *candidate == monitor)
}

fn monitor_at(monitors: &[MonitorBounds], idx: Option<usize>) -> Option<MonitorBounds> {
    idx.and_then(|idx| monitors.get(idx).copied())
}

fn fallback_monitor(monitors: &[MonitorBounds]) -> MonitorBounds {
    monitors.first().copied().unwrap_or(MonitorBounds {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    })
}
