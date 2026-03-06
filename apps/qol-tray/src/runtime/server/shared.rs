use std::collections::HashSet;
use std::sync::mpsc as std_mpsc;
use std::sync::{Mutex, MutexGuard};

use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind};
use qol_runtime::{CursorPos, MonitorBounds, PlatformState, WindowBounds};

use super::super::state::{self, InputState};

pub(super) struct SharedState {
    input: Mutex<InputState>,
    monitors: Mutex<Vec<MonitorBounds>>,
    cursor_pos: Mutex<Option<(f32, f32)>>,
    focused_window: Mutex<Option<MonitorBounds>>,
    last_focus_bounds: Mutex<Option<MonitorBounds>>,
    subscribers: Mutex<Vec<SubscriberEntry>>,
}

struct SubscriberEntry {
    interests: HashSet<RuntimeEventKind>,
    tx: std_mpsc::Sender<RuntimeEvent>,
}

impl SharedState {
    pub(super) fn new(monitors: Vec<MonitorBounds>) -> Self {
        Self {
            input: Mutex::new(InputState::default()),
            monitors: Mutex::new(monitors),
            cursor_pos: Mutex::new(None),
            focused_window: Mutex::new(None),
            last_focus_bounds: Mutex::new(None),
            subscribers: Mutex::new(Vec::new()),
        }
    }

    pub(super) fn add_subscriber(
        &self,
        interests: HashSet<RuntimeEventKind>,
        tx: std_mpsc::Sender<RuntimeEvent>,
    ) {
        let mut subscribers = lock_or_recover(&self.subscribers);
        subscribers.push(SubscriberEntry { interests, tx });
    }

    pub(super) fn build_state(&self) -> PlatformState {
        let monitors = self.monitors();
        let cursor = self.cursor_pos().map(|(x, y)| CursorPos { x, y });
        let input = self.input();

        let cursor_monitor_idx = input.cursor.as_ref().and_then(|cursor| {
            monitors
                .iter()
                .position(|monitor| *monitor == cursor.monitor)
        });

        let focus_monitor_idx = input.focus.as_ref().and_then(|focus| {
            monitors
                .iter()
                .position(|monitor| *monitor == focus.monitor)
        });

        let active = state::pick_active_monitor(&input, fallback_monitor(&monitors));
        let active_monitor_idx = monitors.iter().position(|monitor| *monitor == active);

        log::debug!(
            "[runtime/build_state] GET_STATE cursor_idx={:?} focus_idx={:?} active_idx={:?}",
            cursor_monitor_idx,
            focus_monitor_idx,
            active_monitor_idx
        );

        let focused_window = self.focused_window().map(|bounds| WindowBounds {
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
        });

        PlatformState {
            cursor,
            monitors,
            cursor_monitor_idx,
            focus_monitor_idx,
            active_monitor_idx,
            focused_window,
        }
    }

    pub(super) fn focused_window(&self) -> Option<MonitorBounds> {
        *lock_or_recover(&self.focused_window)
    }

    pub(super) fn has_subscribers(&self) -> bool {
        !lock_or_recover(&self.subscribers).is_empty()
    }

    pub(super) fn input(&self) -> InputState {
        lock_or_recover(&self.input).clone()
    }

    pub(super) fn monitor_at(&self, idx: usize) -> Option<MonitorBounds> {
        lock_or_recover(&self.monitors).get(idx).copied()
    }

    pub(super) fn monitors(&self) -> Vec<MonitorBounds> {
        lock_or_recover(&self.monitors).clone()
    }

    pub(super) fn publish(&self, events: &[RuntimeEvent]) {
        let mut subscribers = lock_or_recover(&self.subscribers);
        subscribers.retain(|entry| publish_to_subscriber(entry, events));
    }

    pub(super) fn remember_focus_bounds(&self, bounds: Option<MonitorBounds>) -> bool {
        let mut last_bounds = lock_or_recover(&self.last_focus_bounds);
        let changed = *last_bounds != bounds;
        if changed {
            *last_bounds = bounds;
        }
        changed
    }

    pub(super) fn set_cursor_pos(&self, cursor_pos: Option<(f32, f32)>) {
        *lock_or_recover(&self.cursor_pos) = cursor_pos;
    }

    pub(super) fn set_monitors(&self, monitors: Vec<MonitorBounds>) {
        *lock_or_recover(&self.monitors) = monitors;
    }

    pub(super) fn store_focused_window(&self, bounds: Option<MonitorBounds>) {
        let Some(bounds) = bounds else {
            return;
        };
        *lock_or_recover(&self.focused_window) = Some(bounds);
    }

    pub(super) fn with_input<T>(&self, update: impl FnOnce(&mut InputState) -> T) -> T {
        let mut input = lock_or_recover(&self.input);
        update(&mut input)
    }

    fn cursor_pos(&self) -> Option<(f32, f32)> {
        *lock_or_recover(&self.cursor_pos)
    }
}

fn fallback_monitor(monitors: &[MonitorBounds]) -> MonitorBounds {
    monitors.first().copied().unwrap_or(MonitorBounds {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    })
}

fn event_kind(event: &RuntimeEvent) -> RuntimeEventKind {
    match event {
        RuntimeEvent::ActiveMonitorChanged { .. } => RuntimeEventKind::ActiveMonitorChanged,
        RuntimeEvent::FocusChanged { .. } => RuntimeEventKind::FocusChanged,
        RuntimeEvent::MonitorsChanged { .. } => RuntimeEventKind::MonitorsChanged,
    }
}

fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

fn publish_to_subscriber(entry: &SubscriberEntry, events: &[RuntimeEvent]) -> bool {
    for event in events {
        if !entry.interests.contains(&event_kind(event)) {
            continue;
        }
        if entry.tx.send(event.clone()).is_err() {
            return false;
        }
    }
    true
}
