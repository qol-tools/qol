mod snapshot;
mod subscribers;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
use std::time::Instant;

use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind};
use qol_runtime::MonitorBounds;

use super::super::state::{self, InputState, Stamped};
use crate::desktop_state::{Platform, SharedPlatform};
use subscribers::{SubscriberEntry, SubscriberId};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Fallback {
    #[default]
    None,
    Pending,
    Holding(MonitorBounds),
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct FocusWindow {
    id: Option<u32>,
    fallback: Fallback,
}

pub(crate) struct SharedState {
    input: Mutex<InputState>,
    monitors: Mutex<Vec<MonitorBounds>>,
    cursor_pos: Mutex<Option<(f32, f32)>>,
    focused_window: Mutex<Option<MonitorBounds>>,
    last_focus_bounds: Mutex<Option<MonitorBounds>>,
    focus_window: Mutex<FocusWindow>,
    subscribers: Mutex<Vec<SubscriberEntry>>,
    subscriber_changed: Condvar,
    next_subscriber_id: AtomicU64,
    armed_lifelines: Mutex<HashMap<String, usize>>,
    platform: OnceLock<SharedPlatform>,
    peers: OnceLock<crate::features::linked_devices::PeerHostHandle>,
}

impl SharedState {
    pub(crate) fn new(monitors: Vec<MonitorBounds>) -> Self {
        Self {
            input: Mutex::new(InputState::default()),
            monitors: Mutex::new(monitors),
            cursor_pos: Mutex::new(None),
            focused_window: Mutex::new(None),
            last_focus_bounds: Mutex::new(None),
            focus_window: Mutex::new(FocusWindow::default()),
            subscribers: Mutex::new(Vec::new()),
            subscriber_changed: Condvar::new(),
            next_subscriber_id: AtomicU64::new(1),
            armed_lifelines: Mutex::new(HashMap::new()),
            platform: OnceLock::new(),
            peers: OnceLock::new(),
        }
    }

    pub(super) fn arm_lifeline(&self, plugin_id: String) {
        *lock_or_recover(&self.armed_lifelines)
            .entry(plugin_id)
            .or_insert(0) += 1;
    }

    pub(super) fn disarm_lifeline(&self, plugin_id: &str) {
        let mut lifelines = lock_or_recover(&self.armed_lifelines);
        if let Some(count) = lifelines.get_mut(plugin_id) {
            *count -= 1;
            if *count == 0 {
                lifelines.remove(plugin_id);
            }
        }
    }

    pub(super) fn armed_lifelines(&self) -> Vec<String> {
        let mut ids: Vec<String> = lock_or_recover(&self.armed_lifelines)
            .keys()
            .cloned()
            .collect();
        ids.sort();
        ids
    }

    pub(crate) fn attach_platform(&self, facade: SharedPlatform) {
        let _ = self.platform.set(facade);
    }

    pub(crate) fn attach_peers(
        &self,
        handle: crate::features::linked_devices::PeerHostHandle,
    ) -> bool {
        self.peers.set(handle).is_ok()
    }

    pub(crate) fn peers(&self) -> Option<&crate::features::linked_devices::PeerHostHandle> {
        self.peers.get()
    }

    #[cfg(any(unix, test))]
    pub(crate) fn peer_admin(
        &self,
        request: qol_peers::admin::Request,
    ) -> qol_peers::admin::Response {
        match self.peers() {
            Some(peers) => peers.request(request),
            None => qol_peers::admin::Response::Error {
                error: qol_peers::admin::Error::HostUnavailable,
            },
        }
    }

    /// Forces inline desktop queries against the OS, bypassing the poll loop's
    /// adaptive interval. Used by GET_STATE and subscription replay so callers
    /// see request-time state even when no poll-driven subscriber is active.
    pub(crate) fn refresh_snapshot_synchronously(&self) {
        let Some(facade) = self.platform.get() else {
            return;
        };
        self.refresh_monitors_synchronously(facade);
        self.refresh_cursor_synchronously(facade);
        self.refresh_focus_synchronously(facade);
    }

    fn refresh_monitors_synchronously(&self, facade: &SharedPlatform) {
        let fresh = facade.physical_monitors();
        if fresh.is_empty() {
            return;
        }
        let mut monitors = lock_or_recover(&self.monitors);
        if *monitors == fresh {
            return;
        }
        *monitors = fresh;
    }

    fn refresh_cursor_synchronously(&self, facade: &SharedPlatform) {
        let fresh = facade.cursor_position();
        self.set_cursor_pos(fresh);
        let Some((x, y)) = fresh else {
            return;
        };
        let monitors = self.monitors();
        let Some(fresh_monitor) = state::monitor_for_point(&monitors, x, y) else {
            return;
        };
        let mut input = lock_or_recover(&self.input);
        if input
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.monitor == fresh_monitor)
        {
            return;
        }
        input.cursor = Some(Stamped {
            monitor: fresh_monitor,
            at: Instant::now(),
        });
    }

    fn refresh_focus_synchronously(&self, facade: &SharedPlatform) {
        if !facade.poll_focused_window() {
            return;
        }
        let Some(focused) = facade.focused_window() else {
            return;
        };
        self.track_focus_window(focused.id, facade.as_ref());
        let fresh_bounds = focused.monitor;
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
        if needs_update && !self.holds_focus(Some(fresh_bounds)) {
            input.focus = Some(Stamped {
                monitor: fresh_monitor,
                at: Instant::now(),
            });
        }
    }

    pub(super) fn add_subscriber(
        &self,
        plugin_id: String,
        interests: HashSet<RuntimeEventKind>,
        tx: std_mpsc::Sender<RuntimeEvent>,
    ) -> SubscriberId {
        let id = self.next_subscriber_id.fetch_add(1, Ordering::Relaxed);
        subscribers::push(&self.subscribers, id, plugin_id, interests, tx);
        self.subscriber_changed.notify_all();
        id
    }

    pub(super) fn remove_subscriber(&self, id: SubscriberId) {
        if subscribers::remove(&self.subscribers, id) {
            self.subscriber_changed.notify_all();
        }
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

    pub(super) fn has_poll_subscribers(&self) -> bool {
        subscribers::has_poll_subscribers(&self.subscribers)
    }

    pub(super) fn has_window_list_subscribers(&self) -> bool {
        subscribers::has_window_list_subscribers(&self.subscribers)
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
        let lifelines = self.armed_lifelines();
        let monitors = self.monitors();
        if subscribers::publish(&self.subscribers, events, &lifelines, &monitors) {
            self.subscriber_changed.notify_all();
        }
    }

    pub(super) fn wait_for_poll_subscriber(&self) {
        let mut subscribers = lock_or_recover(&self.subscribers);
        while !subscribers::has_poll_subscribers_in(&subscribers) {
            subscribers = self
                .subscriber_changed
                .wait(subscribers)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    pub(super) fn wait_for_window_list_subscriber(&self) {
        let mut subscribers = lock_or_recover(&self.subscribers);
        while !subscribers::has_event_subscribers_in(
            &subscribers,
            RuntimeEventKind::WindowListChanged,
        ) {
            subscribers = self
                .subscriber_changed
                .wait(subscribers)
                .unwrap_or_else(|error| error.into_inner());
        }
    }

    pub(super) fn remember_focus_bounds(&self, bounds: Option<MonitorBounds>) -> bool {
        let mut last_bounds = lock_or_recover(&self.last_focus_bounds);
        let changed = *last_bounds != bounds;
        if changed {
            *last_bounds = bounds;
        }
        changed
    }

    pub(super) fn track_focus_window(&self, id: Option<u32>, platform: &dyn Platform) {
        let mut tracked = lock_or_recover(&self.focus_window);
        if id.is_none() || tracked.id == id {
            return;
        }
        let closed = tracked
            .id
            .is_some_and(|previous| platform.window_open(previous) == Some(false));
        *tracked = FocusWindow {
            id,
            fallback: if closed {
                Fallback::Pending
            } else {
                Fallback::None
            },
        };
    }

    pub(super) fn holds_focus(&self, bounds: Option<MonitorBounds>) -> bool {
        let mut tracked = lock_or_recover(&self.focus_window);
        match tracked.fallback {
            Fallback::None => false,
            Fallback::Pending => {
                if let Some(bounds) = bounds {
                    tracked.fallback = Fallback::Holding(bounds);
                }
                true
            }
            Fallback::Holding(held) if Some(held) == bounds => true,
            Fallback::Holding(_) => {
                tracked.fallback = Fallback::None;
                false
            }
        }
    }

    pub(super) fn focus_held(&self) -> bool {
        lock_or_recover(&self.focus_window).fallback != Fallback::None
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

#[cfg(test)]
mod tests {
    use super::*;

    fn interests(kinds: &[RuntimeEventKind]) -> HashSet<RuntimeEventKind> {
        kinds.iter().copied().collect()
    }

    #[test]
    fn remove_subscriber_clears_only_matching_token() {
        let shared = SharedState::new(Vec::new());
        let (poll_tx, _poll_rx) = std_mpsc::channel();
        let (window_tx, _window_rx) = std_mpsc::channel();

        let poll_id = shared.add_subscriber(
            "poll".to_string(),
            interests(&[RuntimeEventKind::CursorMoved]),
            poll_tx,
        );
        let window_id = shared.add_subscriber(
            "window".to_string(),
            interests(&[RuntimeEventKind::WindowListChanged]),
            window_tx,
        );

        assert!(shared.has_poll_subscribers());
        assert!(shared.has_window_list_subscribers());

        shared.remove_subscriber(poll_id);

        assert!(!shared.has_poll_subscribers());
        assert!(shared.has_window_list_subscribers());

        shared.remove_subscriber(window_id);

        assert!(!shared.has_window_list_subscribers());
    }

    #[test]
    fn lifeline_stays_armed_while_an_older_connection_for_the_same_id_drops() {
        let shared = SharedState::new(Vec::new());

        shared.arm_lifeline("qol-monitor".to_string());
        shared.arm_lifeline("qol-monitor".to_string());
        shared.disarm_lifeline("qol-monitor");

        assert_eq!(shared.armed_lifelines(), vec!["qol-monitor".to_string()]);
    }

    #[test]
    fn lifeline_disarms_once_every_connection_has_dropped() {
        let shared = SharedState::new(Vec::new());

        shared.arm_lifeline("qol-monitor".to_string());
        shared.arm_lifeline("qol-monitor".to_string());
        shared.disarm_lifeline("qol-monitor");
        shared.disarm_lifeline("qol-monitor");

        assert!(shared.armed_lifelines().is_empty());
    }

    #[test]
    fn disarming_an_unarmed_id_is_a_no_op() {
        let shared = SharedState::new(Vec::new());

        shared.disarm_lifeline("qol-monitor");

        assert!(shared.armed_lifelines().is_empty());
    }

    struct Windows {
        open: Mutex<Vec<u32>>,
    }

    impl Windows {
        fn new(open: &[u32]) -> Self {
            Self {
                open: Mutex::new(open.to_vec()),
            }
        }

        fn close(&self, id: u32) {
            lock_or_recover(&self.open).retain(|open| *open != id);
        }
    }

    impl Platform for Windows {
        fn cursor_position(&self) -> Option<(f32, f32)> {
            None
        }

        fn focused_window_bounds(&self) -> Option<MonitorBounds> {
            None
        }

        fn physical_monitors(&self) -> Vec<MonitorBounds> {
            Vec::new()
        }

        fn window_open(&self, id: u32) -> Option<bool> {
            Some(lock_or_recover(&self.open).contains(&id))
        }
    }

    fn at(x: f32) -> MonitorBounds {
        MonitorBounds {
            x,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        }
    }

    fn fallen_back(windows: &Windows) -> SharedState {
        let shared = SharedState::new(Vec::new());
        shared.track_focus_window(Some(1), windows);
        windows.close(1);
        shared.track_focus_window(Some(2), windows);
        shared
    }

    #[test]
    fn focus_moved_by_a_closing_window_falls_back() {
        let windows = Windows::new(&[1, 2]);
        let shared = fallen_back(&windows);
        assert!(shared.focus_held());
        assert!(shared.holds_focus(Some(at(0.0))));
    }

    #[test]
    fn focus_switched_between_open_windows_is_deliberate() {
        let shared = SharedState::new(Vec::new());
        let windows = Windows::new(&[1, 2]);
        shared.track_focus_window(Some(1), &windows);
        shared.track_focus_window(Some(2), &windows);
        assert!(!shared.holds_focus(Some(at(0.0))));
    }

    #[test]
    fn a_fallback_ends_at_the_next_focus_switch() {
        let windows = Windows::new(&[1, 2, 3]);
        let shared = fallen_back(&windows);
        shared.track_focus_window(Some(3), &windows);
        assert!(!shared.holds_focus(Some(at(0.0))));
    }

    #[test]
    fn a_window_moved_after_a_fallback_moves_focus() {
        let windows = Windows::new(&[1, 2]);
        let shared = fallen_back(&windows);
        assert!(shared.holds_focus(Some(at(0.0))));
        assert!(shared.holds_focus(Some(at(0.0))));
        assert!(!shared.holds_focus(Some(at(800.0))));
        assert!(!shared.focus_held());
        assert!(!shared.holds_focus(Some(at(0.0))));
    }

    #[test]
    fn an_unanswerable_window_list_never_counts_as_a_fallback() {
        struct Blind;
        impl Platform for Blind {
            fn cursor_position(&self) -> Option<(f32, f32)> {
                None
            }
            fn focused_window_bounds(&self) -> Option<MonitorBounds> {
                None
            }
            fn physical_monitors(&self) -> Vec<MonitorBounds> {
                Vec::new()
            }
        }
        let shared = SharedState::new(Vec::new());
        shared.track_focus_window(Some(1), &Blind);
        shared.track_focus_window(Some(2), &Blind);
        assert!(!shared.focus_held());
    }
}
