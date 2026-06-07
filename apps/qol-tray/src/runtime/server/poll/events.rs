use qol_runtime::protocol::RuntimeEvent;
use qol_runtime::MonitorBounds;

use crate::runtime::server::shared::SharedState;
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
        cursor_moved: bool,
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
        if let Some(event) = cursor_moved_event(shared, cursor_moved) {
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

        #[cfg(debug_assertions)]
        let is_boot = self.prev_active_idx.is_none();
        self.prev_active_idx = current_idx;

        #[cfg(debug_assertions)]
        qol_runtime::probe!("HOST_EMIT_AMC", "new_idx={current_idx:?} is_boot={is_boot}");

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

fn cursor_moved_event(shared: &SharedState, moved: bool) -> Option<RuntimeEvent> {
    if !moved {
        return None;
    }
    let (x, y) = shared.cursor_pos()?;
    Some(RuntimeEvent::CursorMoved { x, y })
}

fn monitors_changed_event(shared: &SharedState, changed: bool) -> Option<RuntimeEvent> {
    if !changed {
        return None;
    }

    let monitors = shared.monitors();
    #[cfg(debug_assertions)]
    qol_runtime::probe!("HOST_EMIT_MONITORS", "n={}", monitors.len());
    Some(RuntimeEvent::MonitorsChanged { monitors })
}

fn active_monitor_idx(input: &InputState, monitors: &[MonitorBounds]) -> Option<usize> {
    let active = state::pick_active_monitor(input).or_else(|| monitors.first().copied())?;
    monitor_idx(monitors, active)
}

fn focus_monitor_idx(input: &InputState, monitors: &[MonitorBounds]) -> Option<usize> {
    let focus = input.focus.as_ref()?;
    monitor_idx(monitors, focus.monitor)
}

fn monitor_idx(monitors: &[MonitorBounds], monitor: MonitorBounds) -> Option<usize> {
    monitors.iter().position(|candidate| *candidate == monitor)
}

fn monitor_at(monitors: &[MonitorBounds], idx: Option<usize>) -> Option<MonitorBounds> {
    idx.and_then(|idx| monitors.get(idx).copied())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    use crate::runtime::state::Stamped;

    fn mon(x: f32, y: f32) -> MonitorBounds {
        MonitorBounds {
            x,
            y,
            width: 1000.0,
            height: 1000.0,
        }
    }

    fn shared_with(monitors: Vec<MonitorBounds>) -> crate::runtime::server::shared::SharedState {
        crate::runtime::server::shared::SharedState::new(monitors)
    }

    fn set_cursor_focus(
        shared: &crate::runtime::server::shared::SharedState,
        cursor: Option<MonitorBounds>,
        focus: Option<MonitorBounds>,
        now: Instant,
    ) {
        shared.with_input(|input| {
            input.cursor = cursor.map(|m| Stamped {
                monitor: m,
                at: now,
            });
            input.focus = focus.map(|m| Stamped {
                monitor: m,
                at: now,
            });
        });
    }

    #[test]
    fn build_emits_only_initial_active_when_state_is_idle_then_settles() {
        let shared = shared_with(vec![mon(0.0, 0.0)]);
        let mut tracker = EventTracker::new();
        // First call: prev_active_idx = None -> Some(0) (via fallback monitor), emit
        // ActiveMonitorChanged once. No focus, no cursor pos, no monitors-changed flag.
        let first = tracker.build(&shared, &[mon(0.0, 0.0)], false, false);
        assert_eq!(
            first.len(),
            1,
            "first build settles initial active idx: {first:?}"
        );
        assert!(matches!(
            first[0],
            RuntimeEvent::ActiveMonitorChanged {
                monitor_idx: Some(0),
                ..
            }
        ));
        // Second call with no state changes: nothing should fire.
        let second = tracker.build(&shared, &[mon(0.0, 0.0)], false, false);
        assert!(
            second.is_empty(),
            "idle settled state emits nothing: {second:?}"
        );
    }

    #[test]
    fn build_emits_active_monitor_changed_only_when_index_changes() {
        let monitors = vec![mon(0.0, 0.0), mon(2000.0, 0.0)];
        let shared = shared_with(monitors.clone());
        set_cursor_focus(&shared, Some(monitors[0]), None, Instant::now());

        let mut tracker = EventTracker::new();
        let first = tracker.build(&shared, &monitors, false, false);
        assert!(
            first.iter().any(|event| matches!(
                event,
                RuntimeEvent::ActiveMonitorChanged {
                    monitor_idx: Some(0),
                    ..
                }
            )),
            "first build emits initial active=Some(0): {first:?}",
        );

        let again = tracker.build(&shared, &monitors, false, false);
        assert!(
            !again
                .iter()
                .any(|event| matches!(event, RuntimeEvent::ActiveMonitorChanged { .. })),
            "second build with unchanged active emits no ActiveMonitorChanged: {again:?}",
        );

        set_cursor_focus(&shared, Some(monitors[1]), None, Instant::now());
        let after = tracker.build(&shared, &monitors, false, false);
        assert!(
            after.iter().any(|event| matches!(
                event,
                RuntimeEvent::ActiveMonitorChanged {
                    monitor_idx: Some(1),
                    ..
                }
            )),
            "monitor switch emits ActiveMonitorChanged(Some(1)): {after:?}",
        );
    }

    #[test]
    fn build_emits_focus_changed_only_when_focus_idx_changes() {
        let monitors = vec![mon(0.0, 0.0), mon(2000.0, 0.0)];
        let shared = shared_with(monitors.clone());
        set_cursor_focus(&shared, None, Some(monitors[0]), Instant::now());
        let mut tracker = EventTracker::new();

        let first = tracker.build(&shared, &monitors, false, false);
        let focus_events: Vec<_> = first
            .iter()
            .filter(|event| matches!(event, RuntimeEvent::FocusChanged { .. }))
            .collect();
        assert_eq!(focus_events.len(), 1, "first build emits FocusChanged");

        let again = tracker.build(&shared, &monitors, false, false);
        let focus_events: Vec<_> = again
            .iter()
            .filter(|event| matches!(event, RuntimeEvent::FocusChanged { .. }))
            .collect();
        assert!(focus_events.is_empty(), "no event when focus idx unchanged");
    }

    #[test]
    fn build_emits_cursor_moved_only_when_moved_flag_set_and_cursor_pos_known() {
        let shared = shared_with(vec![mon(0.0, 0.0)]);
        let mut tracker = EventTracker::new();
        // moved=true but no cursor pos: no event.
        let events = tracker.build(&shared, &[mon(0.0, 0.0)], false, true);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::CursorMoved { .. })),
            "moved=true with no cursor_pos: no event ({events:?})",
        );

        shared.set_cursor_pos(Some((123.0, 456.0)));
        let events = tracker.build(&shared, &[mon(0.0, 0.0)], false, true);
        let cursor_events: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                RuntimeEvent::CursorMoved { x, y } => Some((*x, *y)),
                _ => None,
            })
            .collect();
        assert_eq!(cursor_events, vec![(123.0, 456.0)]);

        let events = tracker.build(&shared, &[mon(0.0, 0.0)], false, false);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::CursorMoved { .. })),
            "moved=false: never emits CursorMoved",
        );
    }

    #[test]
    fn build_emits_monitors_changed_only_when_flag_is_set() {
        let monitors = vec![mon(0.0, 0.0)];
        let shared = shared_with(monitors.clone());
        let mut tracker = EventTracker::new();

        let events = tracker.build(&shared, &monitors, false, false);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::MonitorsChanged { .. })),
            "no flag: no event",
        );

        let events = tracker.build(&shared, &monitors, true, false);
        assert!(
            events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::MonitorsChanged { .. })),
            "flag set: emits MonitorsChanged",
        );
    }
}
