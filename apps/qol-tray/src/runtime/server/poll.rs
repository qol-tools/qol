mod events;
mod sample;

use std::sync::Arc;
use std::time::{Duration, Instant};

use qol_runtime::MonitorBounds;

use super::super::channels::cursor::CursorChannel;
use super::super::channels::focus::FocusChannel;
use super::super::channels::monitors::MonitorsChannel;
use super::super::poller::{AdaptivePoller, BasicStrategy};
use super::super::Channel;
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

    fn emit_events(
        &mut self,
        mon_list: &[MonitorBounds],
        monitors_changed: bool,
        cursor_moved: bool,
    ) {
        if !self.shared.has_subscribers() {
            return;
        }

        let events =
            self.event_tracker
                .build(&self.shared, mon_list, monitors_changed, cursor_moved);
        if events.is_empty() {
            return;
        }

        self.shared.publish(&events);
    }

    fn poll_inputs(&mut self, mon_list: &[MonitorBounds]) -> (bool, bool) {
        let sample = self.sample_inputs(mon_list);
        let cursor_moved = sample.cursor_moved;
        let changed = self
            .shared
            .with_input(|input| apply_updates(input, &sample));
        (changed, cursor_moved)
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
        let (input_changed, cursor_moved) = self.poll_inputs(&mon_list);
        self.emit_events(&mon_list, monitors_changed, cursor_moved);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_state::Platform;
    use crate::runtime::server::shared::SharedState;
    use proptest::prelude::*;
    use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind};
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    type CursorPos = Option<(f32, f32)>;
    type FocusBounds = Option<MonitorBounds>;
    type Monitors = Vec<MonitorBounds>;

    struct ScriptedPlatform {
        cursor: Mutex<Vec<CursorPos>>,
        focus: Mutex<Vec<FocusBounds>>,
        monitors: Mutex<Vec<Monitors>>,
        poll_focus: bool,
    }

    impl ScriptedPlatform {
        fn new(
            cursor: Vec<CursorPos>,
            focus: Vec<FocusBounds>,
            monitors: Vec<Monitors>,
        ) -> Arc<Self> {
            Arc::new(Self {
                cursor: Mutex::new(cursor),
                focus: Mutex::new(focus),
                monitors: Mutex::new(monitors),
                poll_focus: true,
            })
        }
    }

    impl Platform for ScriptedPlatform {
        fn cursor_position(&self) -> Option<(f32, f32)> {
            let mut q = self.cursor.lock().unwrap();
            if q.is_empty() {
                return None;
            }
            q.remove(0)
        }
        fn focused_window_bounds(&self) -> Option<MonitorBounds> {
            let mut q = self.focus.lock().unwrap();
            if q.is_empty() {
                return None;
            }
            q.remove(0)
        }
        fn physical_monitors(&self) -> Vec<MonitorBounds> {
            let mut q = self.monitors.lock().unwrap();
            if q.is_empty() {
                return Vec::new();
            }
            q.remove(0)
        }
        fn poll_focused_window(&self) -> bool {
            self.poll_focus
        }
    }

    fn mon(x: f32) -> MonitorBounds {
        MonitorBounds {
            x,
            y: 0.0,
            width: 1000.0,
            height: 1000.0,
        }
    }

    fn build_runtime(
        cursor_seq: Vec<CursorPos>,
        focus_seq: Vec<FocusBounds>,
        monitor_seq: Vec<Monitors>,
        initial_monitors: Monitors,
    ) -> (Arc<SharedState>, PollRuntime) {
        let platform = ScriptedPlatform::new(cursor_seq, focus_seq, monitor_seq);
        let channels = RuntimeChannels::new(platform);
        let shared = Arc::new(SharedState::new(initial_monitors));
        let runtime = PollRuntime::new(Arc::clone(&shared), channels);
        (shared, runtime)
    }

    fn subscribe_all(shared: &SharedState) -> std::sync::mpsc::Receiver<RuntimeEvent> {
        let (tx, rx) = std::sync::mpsc::channel();
        let interests: HashSet<RuntimeEventKind> = [
            RuntimeEventKind::ActiveMonitorChanged,
            RuntimeEventKind::CursorMoved,
            RuntimeEventKind::FocusChanged,
            RuntimeEventKind::MonitorsChanged,
        ]
        .into_iter()
        .collect();
        shared.add_subscriber(interests, tx);
        rx
    }

    fn drain(rx: &std::sync::mpsc::Receiver<RuntimeEvent>) -> Vec<RuntimeEvent> {
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    #[test]
    fn runtime_channels_initial_monitors_mirrors_platform_first_snapshot() {
        let monitors = vec![mon(0.0), mon(2000.0)];
        let platform = ScriptedPlatform::new(Vec::new(), Vec::new(), vec![monitors.clone()]);
        let channels = RuntimeChannels::new(platform);
        assert_eq!(channels.initial_monitors(), monitors);
    }

    #[test]
    fn runtime_channels_initial_monitors_is_empty_when_platform_returns_empty() {
        let platform = ScriptedPlatform::new(Vec::new(), Vec::new(), vec![Vec::new()]);
        let channels = RuntimeChannels::new(platform);
        assert!(channels.initial_monitors().is_empty());
    }

    #[test]
    fn poll_runtime_new_starts_poller_at_max_interval() {
        let (_shared, runtime) =
            build_runtime(Vec::new(), Vec::new(), vec![vec![mon(0.0)]], vec![mon(0.0)]);
        assert_eq!(runtime.poller.current(), Duration::from_millis(POLL_MAX_MS));
        assert_eq!(
            runtime.commit_threshold,
            Duration::from_millis(COMMIT_THRESHOLD_MS),
        );
        assert_eq!(runtime.monitor_interval, Duration::from_secs(5));
    }

    #[test]
    fn wait_for_monitors_returns_false_when_monitors_present() {
        let (_shared, mut runtime) =
            build_runtime(Vec::new(), Vec::new(), vec![vec![mon(0.0)]], vec![mon(0.0)]);
        assert!(!runtime.wait_for_monitors());
    }

    #[test]
    fn refresh_empty_monitors_no_op_when_platform_returns_empty() {
        let (shared, mut runtime) = build_runtime(
            Vec::new(),
            Vec::new(),
            vec![vec![mon(0.0)], Vec::new()],
            Vec::new(),
        );
        runtime.refresh_empty_monitors();
        assert!(
            shared.monitors().is_empty(),
            "shared remains empty when platform reports empty",
        );
    }

    #[test]
    fn refresh_empty_monitors_seeds_shared_when_platform_returns_monitors() {
        let monitors = vec![mon(0.0), mon(1000.0)];
        let (shared, mut runtime) = build_runtime(
            Vec::new(),
            Vec::new(),
            vec![Vec::new(), monitors.clone()],
            Vec::new(),
        );
        assert!(shared.monitors().is_empty(), "starts empty");
        runtime.refresh_empty_monitors();
        assert_eq!(shared.monitors(), monitors);
    }

    #[test]
    fn refresh_monitors_skipped_when_interval_not_elapsed() {
        let (shared, mut runtime) = build_runtime(
            Vec::new(),
            Vec::new(),
            vec![vec![mon(0.0)], vec![mon(0.0), mon(2000.0)]],
            vec![mon(0.0)],
        );
        let changed = runtime.refresh_monitors();
        assert!(!changed, "elapsed < interval -> skip");
        assert_eq!(
            shared.monitors(),
            vec![mon(0.0)],
            "skip means stored monitors unchanged",
        );
    }

    #[test]
    fn refresh_monitors_polls_when_interval_elapsed_and_pushes_change() {
        let new_monitors = vec![mon(0.0), mon(2000.0)];
        let (shared, mut runtime) = build_runtime(
            Vec::new(),
            Vec::new(),
            vec![vec![mon(0.0)], new_monitors.clone()],
            vec![mon(0.0)],
        );
        runtime.last_monitor_poll = Instant::now() - Duration::from_secs(10);
        let changed = runtime.refresh_monitors();
        assert!(changed, "interval elapsed and platform reports new layout");
        assert_eq!(shared.monitors(), new_monitors);
    }

    #[test]
    fn refresh_monitors_returns_false_when_layout_unchanged_even_after_interval() {
        let layout = vec![mon(0.0)];
        let (shared, mut runtime) = build_runtime(
            Vec::new(),
            Vec::new(),
            vec![layout.clone(), layout.clone()],
            layout.clone(),
        );
        runtime.last_monitor_poll = Instant::now() - Duration::from_secs(10);
        let changed = runtime.refresh_monitors();
        assert!(!changed);
        assert_eq!(shared.monitors(), layout);
    }

    #[test]
    fn poll_inputs_returns_changed_true_when_cursor_moves_to_new_monitor() {
        let monitors = vec![mon(0.0), mon(2000.0)];
        let cursor_seq = vec![Some((10.0, 10.0))];
        let (shared, mut runtime) = build_runtime(
            cursor_seq,
            vec![None],
            vec![monitors.clone()],
            monitors.clone(),
        );
        let (changed, cursor_moved) = runtime.poll_inputs(&monitors);
        assert!(cursor_moved, "first cursor sample is always 'moved'");
        assert!(
            changed,
            "applying a fresh cursor monitor must report change"
        );
        assert_eq!(
            shared.cursor_pos(),
            Some((10.0, 10.0)),
            "shared cursor pos written through",
        );
    }

    #[test]
    fn poll_inputs_does_not_stamp_input_cursor_when_cursor_outside_all_monitors() {
        let monitors = vec![mon(0.0)];
        let cursor_seq = vec![Some((9999.0, 9999.0))];
        let (shared, mut runtime) = build_runtime(
            cursor_seq,
            vec![None],
            vec![monitors.clone()],
            monitors.clone(),
        );
        let (_changed, cursor_moved) = runtime.poll_inputs(&monitors);
        assert!(cursor_moved, "channel still reports moved on first sample");
        assert!(
            shared.input().cursor.is_none(),
            "cursor outside any monitor: no input cursor stamp recorded",
        );
    }

    #[test]
    fn poll_inputs_returns_changed_false_when_no_cursor_movement_and_no_focus() {
        let monitors = vec![mon(0.0)];
        let (_shared, mut runtime) = build_runtime(
            vec![None],
            vec![None],
            vec![monitors.clone()],
            monitors.clone(),
        );
        let (changed, cursor_moved) = runtime.poll_inputs(&monitors);
        assert!(!cursor_moved);
        assert!(
            !changed,
            "no cursor pos, no focus, no movement ⇒ apply_updates returns false",
        );
    }

    #[test]
    fn poll_inputs_focus_change_writes_focused_window_and_stamps_input() {
        let monitors = vec![mon(0.0)];
        let focus_bounds = MonitorBounds {
            x: 5.0,
            y: 5.0,
            width: 100.0,
            height: 100.0,
        };
        let (shared, mut runtime) = build_runtime(
            vec![None],
            vec![Some(focus_bounds)],
            vec![monitors.clone()],
            monitors.clone(),
        );
        let _ = runtime.poll_inputs(&monitors);
        assert_eq!(
            shared.focused_window(),
            Some(focus_bounds),
            "focused_window mirrored",
        );
        assert!(
            shared.input().focus.is_some(),
            "focus monitor resolves and gets stamped",
        );
    }

    #[test]
    fn emit_events_no_op_when_no_subscribers() {
        let monitors = vec![mon(0.0)];
        let (shared, mut runtime) = build_runtime(
            Vec::new(),
            Vec::new(),
            vec![monitors.clone()],
            monitors.clone(),
        );
        runtime.emit_events(&monitors, true, true);
        assert!(
            !shared.has_subscribers(),
            "subscribers stay zero, no panics",
        );
    }

    #[test]
    fn emit_events_publishes_built_events_to_subscriber() {
        let monitors = vec![mon(0.0)];
        let (shared, mut runtime) = build_runtime(
            Vec::new(),
            Vec::new(),
            vec![monitors.clone()],
            monitors.clone(),
        );
        let rx = subscribe_all(&shared);
        runtime.emit_events(&monitors, true, false);
        let events = drain(&rx);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, RuntimeEvent::MonitorsChanged { .. })),
            "monitors_changed=true must publish MonitorsChanged: {events:?}",
        );
    }

    #[test]
    fn emit_events_skips_publish_when_event_tracker_yields_empty() {
        let monitors = vec![mon(0.0)];
        let (shared, mut runtime) = build_runtime(
            Vec::new(),
            Vec::new(),
            vec![monitors.clone()],
            monitors.clone(),
        );
        let rx = subscribe_all(&shared);
        let _ = runtime
            .event_tracker
            .build(&shared, &monitors, false, false);
        runtime.emit_events(&monitors, false, false);
        assert!(
            drain(&rx).is_empty(),
            "no flags + settled tracker = no events"
        );
    }

    #[test]
    fn sample_inputs_records_cursor_pos_and_resolves_monitor_indices() {
        let monitors = vec![mon(0.0), mon(2000.0)];
        let focus_bounds = MonitorBounds {
            x: 2010.0,
            y: 10.0,
            width: 100.0,
            height: 100.0,
        };
        let (shared, mut runtime) = build_runtime(
            vec![Some((50.0, 50.0))],
            vec![Some(focus_bounds)],
            vec![monitors.clone()],
            monitors.clone(),
        );
        let sample = runtime.sample_inputs(&monitors);
        assert!(sample.cursor_moved);
        assert!(sample.focus_changed);
        assert_eq!(sample.cursor_monitor, Some(monitors[0]));
        assert_eq!(sample.focus_monitor, Some(monitors[1]));
        assert_eq!(sample.focus_bounds, Some(focus_bounds));
        assert_eq!(shared.cursor_pos(), Some((50.0, 50.0)));
        assert_eq!(shared.focused_window(), Some(focus_bounds));
    }

    #[test]
    fn sample_inputs_committed_flag_reflects_poller_threshold() {
        let monitors = vec![mon(0.0)];
        let (_shared, mut runtime) = build_runtime(
            vec![None],
            vec![None],
            vec![monitors.clone()],
            monitors.clone(),
        );
        let sample = runtime.sample_inputs(&monitors);
        assert!(
            sample.committed,
            "poller starts at MAX (500ms) which is >= COMMIT_THRESHOLD ({COMMIT_THRESHOLD_MS}ms)",
        );

        for _ in 0..16 {
            runtime.poller.tick(true);
        }
        assert_eq!(runtime.poller.current(), Duration::from_millis(POLL_MIN_MS));
        let sample = runtime.sample_inputs(&monitors);
        assert!(
            !sample.committed,
            "POLL_MIN ({POLL_MIN_MS}ms) is below COMMIT_THRESHOLD ({COMMIT_THRESHOLD_MS}ms)",
        );
    }

    #[test]
    fn sample_inputs_remember_focus_bounds_only_marks_changed_on_distinct_bounds() {
        let monitors = vec![mon(0.0)];
        let bounds_a = MonitorBounds {
            x: 10.0,
            y: 10.0,
            width: 100.0,
            height: 100.0,
        };
        let bounds_b = MonitorBounds {
            x: 200.0,
            y: 10.0,
            width: 100.0,
            height: 100.0,
        };
        let (_shared, mut runtime) = build_runtime(
            vec![None, None, None],
            vec![Some(bounds_a), Some(bounds_a), Some(bounds_b)],
            vec![monitors.clone()],
            monitors.clone(),
        );
        assert!(
            runtime.sample_inputs(&monitors).focus_changed,
            "first focus bounds: changed",
        );
        assert!(
            !runtime.sample_inputs(&monitors).focus_changed,
            "same bounds twice: not changed",
        );
        assert!(
            runtime.sample_inputs(&monitors).focus_changed,
            "distinct bounds: changed",
        );
    }

    #[test]
    fn tick_returns_poll_interval_and_advances_poller_state() {
        let monitors = vec![mon(0.0)];
        let (_shared, mut runtime) = build_runtime(
            vec![Some((50.0, 50.0)), Some((50.0, 50.0))],
            vec![None, None],
            vec![monitors.clone()],
            monitors.clone(),
        );
        let initial = runtime.poller.current();
        let returned = runtime.tick();
        assert_eq!(
            returned,
            runtime.poller.current(),
            "tick returns new interval"
        );
        assert!(
            runtime.poller.current() <= Duration::from_millis(POLL_MAX_MS),
            "must remain bounded",
        );
        assert!(
            runtime.poller.current() >= Duration::from_millis(POLL_MIN_MS),
            "must remain bounded below",
        );
        assert!(returned <= initial, "first tick after change halves");
    }

    #[test]
    fn tick_publishes_events_to_subscribers_when_state_settles() {
        let monitors = vec![mon(0.0), mon(2000.0)];
        let (shared, mut runtime) = build_runtime(
            vec![Some((10.0, 10.0))],
            vec![None],
            vec![monitors.clone()],
            monitors.clone(),
        );
        let rx = subscribe_all(&shared);
        let _ = runtime.tick();
        let events = drain(&rx);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, RuntimeEvent::ActiveMonitorChanged { .. })),
            "first tick with cursor settles ActiveMonitorChanged: {events:?}",
        );
    }

    #[test]
    fn tick_unchanged_doubles_poller_interval_until_max() {
        let monitors = vec![mon(0.0)];
        let (_shared, mut runtime) = build_runtime(
            std::iter::repeat_n(None, 16).collect(),
            std::iter::repeat_n(None, 16).collect(),
            vec![monitors.clone()],
            monitors.clone(),
        );
        for _ in 0..8 {
            runtime.tick();
        }
        assert_eq!(
            runtime.poller.current(),
            Duration::from_millis(POLL_MAX_MS),
            "no input change ⇒ poller climbs to MAX",
        );
    }

    type RefreshCase = (
        &'static str,
        Vec<MonitorBounds>,
        Vec<MonitorBounds>,
        Duration,
        bool,
        Vec<MonitorBounds>,
    );

    #[test]
    fn refresh_monitors_table() {
        let one = vec![mon(0.0)];
        let two = vec![mon(0.0), mon(2000.0)];
        let three = vec![mon(0.0), mon(2000.0), mon(4000.0)];
        let cases: &[RefreshCase] = &[
            (
                "elapsed=0 short-circuits regardless of layout change",
                one.clone(),
                two.clone(),
                Duration::from_millis(0),
                false,
                one.clone(),
            ),
            (
                "elapsed > interval, layout grows",
                one.clone(),
                two.clone(),
                Duration::from_secs(10),
                true,
                two.clone(),
            ),
            (
                "elapsed > interval, layout shrinks",
                two.clone(),
                one.clone(),
                Duration::from_secs(10),
                true,
                one.clone(),
            ),
            (
                "elapsed > interval, layout swap",
                two.clone(),
                three.clone(),
                Duration::from_secs(10),
                true,
                three.clone(),
            ),
            (
                "elapsed > interval, identical layout returns false",
                two.clone(),
                two.clone(),
                Duration::from_secs(10),
                false,
                two.clone(),
            ),
            (
                "elapsed > interval but platform reports empty stays unchanged",
                one.clone(),
                Vec::new(),
                Duration::from_secs(10),
                false,
                one.clone(),
            ),
        ];

        for (label, initial, fresh, elapsed, exp_changed, exp_state) in cases {
            let (shared, mut runtime) = build_runtime(
                Vec::new(),
                Vec::new(),
                vec![initial.clone(), fresh.clone()],
                initial.clone(),
            );
            runtime.last_monitor_poll = Instant::now() - *elapsed;
            let changed = runtime.refresh_monitors();
            assert_eq!(changed, *exp_changed, "case: {label}");
            assert_eq!(shared.monitors(), *exp_state, "case state: {label}");
        }
    }

    fn cursor_seq_strategy() -> impl Strategy<Value = Vec<CursorPos>> {
        proptest::collection::vec(
            prop_oneof![
                Just(None),
                (0.0f32..2000.0, 0.0f32..1000.0).prop_map(|(x, y)| Some((x, y))),
            ],
            1..16,
        )
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_tick_keeps_poller_within_bounds(
            cursor_seq in cursor_seq_strategy(),
        ) {
            let monitors = vec![mon(0.0)];
            let n = cursor_seq.len();
            let focus_seq = vec![None; n];
            let (_shared, mut runtime) = build_runtime(
                cursor_seq,
                focus_seq,
                vec![monitors.clone()],
                monitors.clone(),
            );
            for _ in 0..n {
                let returned = runtime.tick();
                prop_assert!(returned >= Duration::from_millis(POLL_MIN_MS));
                prop_assert!(returned <= Duration::from_millis(POLL_MAX_MS));
                prop_assert_eq!(returned, runtime.poller.current());
            }
        }

        #[test]
        fn prop_sample_inputs_writes_cursor_pos_through_to_shared(
            x in 0.0f32..1000.0,
            y in 0.0f32..1000.0,
        ) {
            let monitors = vec![mon(0.0)];
            let (shared, mut runtime) = build_runtime(
                vec![Some((x, y))],
                vec![None],
                vec![monitors.clone()],
                monitors.clone(),
            );
            let _ = runtime.sample_inputs(&monitors);
            prop_assert_eq!(shared.cursor_pos(), Some((x, y)));
        }

        #[test]
        fn prop_refresh_monitors_skips_below_interval(
            elapsed_ms in 0u64..4_000,
        ) {
            let initial = vec![mon(0.0)];
            let fresh = vec![mon(0.0), mon(2000.0)];
            let (shared, mut runtime) = build_runtime(
                Vec::new(),
                Vec::new(),
                vec![initial.clone(), fresh.clone()],
                initial.clone(),
            );
            runtime.last_monitor_poll = Instant::now() - Duration::from_millis(elapsed_ms);
            let changed = runtime.refresh_monitors();
            prop_assert!(!changed, "elapsed < 5s ⇒ no poll");
            prop_assert_eq!(shared.monitors(), initial.clone());
        }

        #[test]
        fn prop_emit_events_never_publishes_when_no_subscribers(
            cursor_moved in any::<bool>(),
            monitors_changed in any::<bool>(),
        ) {
            let monitors = vec![mon(0.0)];
            let (shared, mut runtime) = build_runtime(
                Vec::new(),
                Vec::new(),
                vec![monitors.clone()],
                monitors.clone(),
            );
            let rx = subscribe_all(&shared);
            drop(rx);
            runtime.emit_events(&monitors, monitors_changed, cursor_moved);
            prop_assert!(true, "no panic across all flag combinations");
        }

        #[test]
        fn prop_poll_inputs_with_no_input_returns_no_change(
            n in 1usize..8,
        ) {
            let monitors = vec![mon(0.0)];
            let (_shared, mut runtime) = build_runtime(
                vec![None; n],
                vec![None; n],
                vec![monitors.clone()],
                monitors.clone(),
            );
            for _ in 0..n {
                let (changed, cursor_moved) = runtime.poll_inputs(&monitors);
                prop_assert!(!changed, "None cursor + None focus ⇒ no apply");
                prop_assert!(!cursor_moved);
            }
        }
    }

    #[test]
    fn integration_tick_then_focus_change_emits_focus_changed_event() {
        let monitors = vec![mon(0.0), mon(2000.0)];
        let focus_a = MonitorBounds {
            x: 10.0,
            y: 10.0,
            width: 50.0,
            height: 50.0,
        };
        let focus_b = MonitorBounds {
            x: 2010.0,
            y: 10.0,
            width: 50.0,
            height: 50.0,
        };
        let (shared, mut runtime) = build_runtime(
            vec![None, None],
            vec![Some(focus_a), Some(focus_b)],
            vec![monitors.clone()],
            monitors.clone(),
        );
        let rx = subscribe_all(&shared);

        let _ = runtime.tick();
        let _ = runtime.tick();

        let events = drain(&rx);
        let focus_changes: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                RuntimeEvent::FocusChanged { monitor_idx, .. } => Some(*monitor_idx),
                _ => None,
            })
            .collect();
        assert_eq!(focus_changes, vec![Some(0), Some(1)]);
    }
}
