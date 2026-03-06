use std::sync::Arc;
use std::time::{Duration, Instant};

use qol_runtime::protocol::RuntimeEvent;
use qol_runtime::MonitorBounds;

use super::super::channel::Channel;
use super::super::channels::cursor::CursorChannel;
use super::super::channels::focus::FocusChannel;
use super::super::channels::monitors::MonitorsChannel;
use super::super::poller::{AdaptivePoller, BasicStrategy};
use super::super::state::{self, InputState, Stamped};
use super::shared::SharedState;
use crate::desktop_state::SharedPlatform;

const POLL_MIN_MS: u64 = 16;
const POLL_MAX_MS: u64 = 500;
const COMMIT_THRESHOLD_MS: u64 = 128;

type MonitorStamp = Option<(MonitorBounds, Instant)>;

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
    prev_active_idx: Option<usize>,
    prev_focus_idx: Option<usize>,
}

struct TickSample {
    committed: bool,
    cursor_monitor: Option<MonitorBounds>,
    cursor_moved: bool,
    focus_bounds: Option<MonitorBounds>,
    focus_changed: bool,
    focus_monitor: Option<MonitorBounds>,
    now: Instant,
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
            prev_active_idx: None,
            prev_focus_idx: None,
        }
    }

    fn build_events(
        &mut self,
        mon_list: &[MonitorBounds],
        monitors_changed: bool,
    ) -> Vec<RuntimeEvent> {
        let input = self.shared.input();
        let active = state::pick_active_monitor(&input, fallback_monitor(mon_list));
        let current_active_idx = mon_list.iter().position(|monitor| *monitor == active);
        let current_focus_idx = input.focus.as_ref().and_then(|focus| {
            mon_list
                .iter()
                .position(|monitor| *monitor == focus.monitor)
        });

        let mut events = Vec::new();

        if monitors_changed {
            events.push(RuntimeEvent::MonitorsChanged {
                monitors: self.shared.monitors(),
            });
        }

        if current_active_idx != self.prev_active_idx {
            events.push(RuntimeEvent::ActiveMonitorChanged {
                monitor_idx: current_active_idx,
                monitor: current_active_idx.and_then(|idx| mon_list.get(idx).copied()),
            });
            self.prev_active_idx = current_active_idx;
        }

        if current_focus_idx != self.prev_focus_idx {
            events.push(RuntimeEvent::FocusChanged {
                monitor_idx: current_focus_idx,
                monitor: current_focus_idx.and_then(|idx| mon_list.get(idx).copied()),
            });
            self.prev_focus_idx = current_focus_idx;
        }

        events
    }

    fn emit_events(&mut self, mon_list: &[MonitorBounds], monitors_changed: bool) {
        if !self.shared.has_subscribers() {
            return;
        }

        let events = self.build_events(mon_list, monitors_changed);
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

fn apply_updates(input: &mut InputState, sample: &TickSample) -> bool {
    let before = snapshot_input(input);
    let cursor_changed = apply_cursor_update(input, sample);
    let focus_changed = apply_focus_update(input, sample);
    log_state_change(input, before, sample);
    sample.cursor_moved || cursor_changed || focus_changed
}

fn fallback_monitor(monitors: &[MonitorBounds]) -> MonitorBounds {
    monitors.first().copied().unwrap_or(MonitorBounds {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    })
}

fn log_state_change(input: &InputState, before: (MonitorStamp, MonitorStamp), sample: &TickSample) {
    let after = snapshot_input(input);
    if before == after {
        return;
    }

    let active = state::pick_active_monitor(input, zero_monitor());

    log::debug!(
        "[runtime/poll] STATE CHANGE committed={} focus_changed={} focus_bounds={:?} cursor=({:?}) focus=({:?}) → active=({}, {})",
        sample.committed,
        sample.focus_changed,
        sample.focus_bounds.map(|b| (b.x, b.y, b.width, b.height)),
        input.cursor.as_ref().map(|c| (c.monitor.x, c.monitor.y)),
        input.focus.as_ref().map(|f| (f.monitor.x, f.monitor.y)),
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
