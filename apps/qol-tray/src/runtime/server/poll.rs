mod events;
mod sample;

use std::sync::Arc;
use std::time::{Duration, Instant};

use qol_runtime::MonitorBounds;

use super::super::channel::Channel;
use super::super::channels::cursor::CursorChannel;
use super::super::channels::focus::FocusChannel;
use super::super::channels::monitors::MonitorsChannel;
use super::super::poller::{AdaptivePoller, BasicStrategy};
use super::shared::SharedState;
use crate::desktop_state::SharedPlatform;
use crate::runtime::state;
use events::EventTracker;
use sample::{apply_updates, TickSample};

const POLL_MIN_MS: u64 = 16;
const POLL_MAX_MS: u64 = 500;
const COMMIT_THRESHOLD_MS: u64 = 128;

pub(super) struct RuntimeChannels {
    cursor: CursorChannel,
    monitors: MonitorsChannel,
    focus: FocusChannel,
}

impl RuntimeChannels {
    pub(super) fn new(platform: SharedPlatform) -> Self {
        Self {
            cursor: CursorChannel::new(platform.clone()),
            monitors: MonitorsChannel::new(platform.clone()),
            focus: FocusChannel::new(platform),
        }
    }

    pub(super) fn initial_monitors(&self) -> Vec<MonitorBounds> {
        self.monitors.monitors().to_vec()
    }
}

pub(super) fn run(shared: Arc<SharedState>, channels: RuntimeChannels) {
    PollRuntime::new(shared, channels).run();
}

struct PollRuntime {
    shared: Arc<SharedState>,
    cursor: CursorChannel,
    monitors: MonitorsChannel,
    focus: FocusChannel,
    poller: AdaptivePoller,
    commit_threshold: Duration,
    last_monitor_poll: Instant,
    monitor_interval: Duration,
    event_tracker: EventTracker,
}

impl PollRuntime {
    fn new(shared: Arc<SharedState>, channels: RuntimeChannels) -> Self {
        let monitor_interval = channels.monitors.min_interval();

        Self {
            shared,
            cursor: channels.cursor,
            monitors: channels.monitors,
            focus: channels.focus,
            poller: AdaptivePoller::new(
                Duration::from_millis(POLL_MIN_MS),
                Duration::from_millis(POLL_MAX_MS),
                Box::new(BasicStrategy),
            ),
            commit_threshold: Duration::from_millis(COMMIT_THRESHOLD_MS),
            last_monitor_poll: Instant::now(),
            monitor_interval,
            event_tracker: EventTracker::new(),
        }
    }

    fn emit_events(&mut self, mon_list: &[MonitorBounds], monitors_changed: bool) {
        if !self.shared.has_subscribers() {
            return;
        }

        let events = self
            .event_tracker
            .build(&self.shared, mon_list, monitors_changed);
        if events.is_empty() {
            return;
        }

        self.shared.publish(&events);
    }

    fn poll_inputs(&mut self, mon_list: &[MonitorBounds]) -> bool {
        let sample = self.sample_inputs(mon_list);
        self.shared
            .with_input(|input| apply_updates(input, &sample))
    }

    fn refresh_empty_monitors(&mut self) {
        if !self.monitors.poll() {
            return;
        }
        self.shared.set_monitors(self.monitors.monitors().to_vec());
    }

    fn refresh_monitors(&mut self) -> bool {
        if self.last_monitor_poll.elapsed() < self.monitor_interval {
            return false;
        }

        let changed = self.monitors.poll();
        if changed {
            self.shared.set_monitors(self.monitors.monitors().to_vec());
        }

        self.last_monitor_poll = Instant::now();
        changed
    }

    fn run(mut self) {
        loop {
            if self.wait_for_monitors() {
                continue;
            }

            let interval = self.tick();
            std::thread::sleep(interval);
        }
    }

    fn sample_inputs(&mut self, mon_list: &[MonitorBounds]) -> TickSample {
        let now = Instant::now();
        let cursor_moved = self.cursor.poll();
        let cursor_pos = self.cursor.position();
        self.shared.set_cursor_pos(cursor_pos);
        let cursor_monitor = cursor_pos.and_then(|(x, y)| state::monitor_for_point(mon_list, x, y));

        self.focus.poll();
        let focus_bounds = self.focus.bounds();
        self.shared.store_focused_window(focus_bounds);
        let focus_monitor =
            focus_bounds.and_then(|bounds| state::monitor_for_bounds(mon_list, &bounds));
        let focus_changed = self.shared.remember_focus_bounds(focus_bounds);

        TickSample {
            committed: self.poller.current() >= self.commit_threshold,
            cursor_monitor,
            cursor_moved,
            focus_bounds,
            focus_changed,
            focus_monitor,
            now,
        }
    }

    fn tick(&mut self) -> Duration {
        let mon_list = self.shared.monitors();
        let monitors_changed = self.refresh_monitors();
        let input_changed = self.poll_inputs(&mon_list);
        self.emit_events(&mon_list, monitors_changed);
        self.poller.tick(input_changed)
    }

    fn wait_for_monitors(&mut self) -> bool {
        if !self.shared.monitors().is_empty() {
            return false;
        }

        std::thread::sleep(Duration::from_secs(1));
        self.refresh_empty_monitors();
        true
    }
}
