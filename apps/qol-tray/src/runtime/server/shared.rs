mod snapshot;
mod subscribers;

use std::collections::HashSet;
use std::sync::mpsc as std_mpsc;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Instant;

use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind};
use qol_runtime::MonitorBounds;

use super::super::state::{self, InputState, Stamped};
use crate::desktop_state::SharedPlatform;
use subscribers::SubscriberEntry;

pub(crate) struct SharedState {
    input: Mutex<InputState>,
    monitors: Mutex<Vec<MonitorBounds>>,
    cursor_pos: Mutex<Option<(f32, f32)>>,
    focused_window: Mutex<Option<MonitorBounds>>,
    last_focus_bounds: Mutex<Option<MonitorBounds>>,
    subscribers: Mutex<Vec<SubscriberEntry>>,
    armed_lifelines: Mutex<HashSet<String>>,
    platform: OnceLock<SharedPlatform>,
}

impl SharedState {
    pub(crate) fn new(monitors: Vec<MonitorBounds>) -> Self {
        Self {
            input: Mutex::new(InputState::default()),
            monitors: Mutex::new(monitors),
            cursor_pos: Mutex::new(None),
            focused_window: Mutex::new(None),
            last_focus_bounds: Mutex::new(None),
            subscribers: Mutex::new(Vec::new()),
            armed_lifelines: Mutex::new(HashSet::new()),
            platform: OnceLock::new(),
        }
    }

    pub(super) fn arm_lifeline(&self, plugin_id: String) {
        lock_or_recover(&self.armed_lifelines).insert(plugin_id);
    }

    pub(super) fn disarm_lifeline(&self, plugin_id: &str) {
        lock_or_recover(&self.armed_lifelines).remove(plugin_id);
    }

    pub(super) fn armed_lifelines(&self) -> Vec<String> {
        let mut ids: Vec<String> = lock_or_recover(&self.armed_lifelines)
            .iter()
            .cloned()
            .collect();
        ids.sort();
        ids
    }

    pub(crate) fn attach_platform(&self, facade: SharedPlatform) {
        let _ = self.platform.set(facade);
    }

    /// Forces an inline AX focus query against the OS, bypassing the poll
    /// loop's adaptive interval. Used by GET_STATE so callers (popup placement,
    /// alt-tab) see the OS focus at request time, not the last poll tick.
    /// No-op when no facade is attached (test builds).
    pub(crate) fn refresh_focus_synchronously(&self) {
        let Some(facade) = self.platform.get() else {
            return;
        };
        if !facade.poll_focused_window() {
            return;
        }
        let Some(fresh_bounds) = facade.focused_window_bounds() else {
            return;
        };
        let monitors = self.monitors();
        let Some(fresh_monitor) = state::monitor_for_bounds(&monitors, &fresh_bounds) else {
            return;
        };
        *lock_or_recover(&self.focused_window) = Some(fresh_bounds);
        *lock_or_recover(&self.last_focus_bounds) = Some(fresh_bounds);
        let mut input = lock_or_recover(&self.input);
        let needs_update = match input.focus.as_ref() {
            Some(focus) => focus.monitor != fresh_monitor,
            None => true,
        };
        if needs_update {
            input.focus = Some(Stamped {
                monitor: fresh_monitor,
                at: Instant::now(),
            });
        }
    }

    pub(super) fn add_subscriber(
        &self,
        interests: HashSet<RuntimeEventKind>,
        tx: std_mpsc::Sender<RuntimeEvent>,
    ) {
        subscribers::push(&self.subscribers, interests, tx);
    }

    pub(super) fn build_state(&self) -> qol_runtime::PlatformState {
        snapshot::build_state(self)
    }

    pub(super) fn focused_window(&self) -> Option<MonitorBounds> {
        *lock_or_recover(&self.focused_window)
    }

    pub(super) fn has_subscribers(&self) -> bool {
        subscribers::has_subscribers(&self.subscribers)
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

    pub(crate) fn publish(&self, events: &[RuntimeEvent]) {
        subscribers::publish(&self.subscribers, events);
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

    pub(super) fn cursor_pos(&self) -> Option<(f32, f32)> {
        *lock_or_recover(&self.cursor_pos)
    }
}

pub(super) fn lock_or_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}
