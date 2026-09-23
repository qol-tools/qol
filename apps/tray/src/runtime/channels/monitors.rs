use std::time::Duration;

use super::super::Channel;
use crate::desktop_state::SharedPlatform;

const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

pub(crate) struct MonitorsChannel {
    platform: SharedPlatform,
    monitors: Vec<qol_runtime::MonitorBounds>,
}

impl MonitorsChannel {
    pub(crate) fn new(platform: SharedPlatform) -> Self {
        let monitors = platform.physical_monitors();
        Self { platform, monitors }
    }

    pub(crate) fn monitors(&self) -> &[qol_runtime::MonitorBounds] {
        &self.monitors
    }
}

impl Channel for MonitorsChannel {
    fn poll(&mut self) -> bool {
        let fresh = self.platform.physical_monitors();
        if !fresh.is_empty() && fresh != self.monitors {
            self.monitors = fresh;
            return true;
        }
        false
    }

    fn min_interval(&self) -> Duration {
        REFRESH_INTERVAL
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use proptest::prelude::*;
    use qol_runtime::MonitorBounds;

    use super::super::super::Channel;
    use super::*;
    use crate::desktop_state::Platform;

    struct ScriptedPlatform {
        snapshots: Mutex<Vec<Vec<MonitorBounds>>>,
    }

    impl ScriptedPlatform {
        fn new(snapshots: Vec<Vec<MonitorBounds>>) -> Arc<Self> {
            Arc::new(Self {
                snapshots: Mutex::new(snapshots),
            })
        }
    }

    impl Platform for ScriptedPlatform {
        fn cursor_position(&self) -> Option<(f32, f32)> {
            None
        }
        fn focused_window_bounds(&self) -> Option<MonitorBounds> {
            None
        }
        fn physical_monitors(&self) -> Vec<MonitorBounds> {
            let mut q = self.snapshots.lock().unwrap();
            if q.is_empty() {
                return Vec::new();
            }
            q.remove(0)
        }
    }

    fn m(x: f32, y: f32, w: f32, h: f32) -> MonitorBounds {
        MonitorBounds {
            x,
            y,
            width: w,
            height: h,
        }
    }

    fn build(snapshots: Vec<Vec<MonitorBounds>>) -> MonitorsChannel {
        MonitorsChannel::new(ScriptedPlatform::new(snapshots))
    }

    #[test]
    fn min_interval_is_5_seconds() {
        let ch = MonitorsChannel::new(ScriptedPlatform::new(Vec::new()));
        assert_eq!(ch.min_interval(), Duration::from_secs(5));
    }

    #[test]
    fn new_seeds_monitors_from_initial_query() {
        let initial = vec![m(0.0, 0.0, 1920.0, 1080.0), m(1920.0, 0.0, 1280.0, 720.0)];
        let platform = ScriptedPlatform::new(vec![initial.clone()]);
        let ch = MonitorsChannel::new(platform);
        assert_eq!(ch.monitors(), initial.as_slice());
    }

    #[test]
    fn new_with_empty_initial_returns_empty_slice() {
        let platform = ScriptedPlatform::new(vec![Vec::new()]);
        let ch = MonitorsChannel::new(platform);
        assert!(ch.monitors().is_empty());
    }

    type PollCase = (
        &'static str,
        Vec<Vec<MonitorBounds>>,
        Vec<bool>,
        Vec<Vec<MonitorBounds>>,
    );

    #[test]
    fn poll_transitions_table() {
        let s1 = vec![m(0.0, 0.0, 100.0, 100.0)];
        let s1b = vec![m(0.0, 0.0, 100.0, 100.0)];
        let s2 = vec![m(0.0, 0.0, 200.0, 200.0)];
        let s_two = vec![m(0.0, 0.0, 100.0, 100.0), m(100.0, 0.0, 100.0, 100.0)];
        let cases: &[PollCase] = &[
            (
                "identical snapshot is no change",
                vec![s1.clone(), s1b.clone()],
                vec![false],
                vec![s1.clone()],
            ),
            (
                "different bounds triggers change",
                vec![s1.clone(), s2.clone()],
                vec![true],
                vec![s2.clone()],
            ),
            (
                "added monitor triggers change",
                vec![s1.clone(), s_two.clone()],
                vec![true],
                vec![s_two.clone()],
            ),
            (
                "removed monitor triggers change",
                vec![s_two.clone(), s1.clone()],
                vec![true],
                vec![s1.clone()],
            ),
            (
                "empty snapshot does not clear stored monitors",
                vec![s1.clone(), Vec::new()],
                vec![false],
                vec![s1.clone()],
            ),
            (
                "empty then different non-empty triggers change",
                vec![s1.clone(), Vec::new(), s2.clone()],
                vec![false, true],
                vec![s1.clone(), s2.clone()],
            ),
            (
                "different then same returns true then false",
                vec![s1.clone(), s2.clone(), s2.clone()],
                vec![true, false],
                vec![s2.clone(), s2.clone()],
            ),
        ];

        for (label, snapshots, exp_changed, exp_states) in cases {
            let initial = snapshots[0].clone();
            let platform = ScriptedPlatform::new(snapshots.clone());
            let mut ch = MonitorsChannel::new(platform);
            assert_eq!(ch.monitors(), initial.as_slice(), "initial ({label})");

            let mut got_changed = Vec::with_capacity(snapshots.len() - 1);
            let mut got_states = Vec::with_capacity(snapshots.len() - 1);
            for _ in 1..snapshots.len() {
                got_changed.push(ch.poll());
                got_states.push(ch.monitors().to_vec());
            }
            assert_eq!(&got_changed, exp_changed, "changed ({label})");
            assert_eq!(&got_states, exp_states, "states ({label})");
        }
    }

    #[test]
    fn order_difference_with_same_set_is_a_change() {
        let a = m(0.0, 0.0, 100.0, 100.0);
        let b = m(100.0, 0.0, 100.0, 100.0);
        let snapshots = vec![vec![a, b], vec![b, a]];
        let platform = ScriptedPlatform::new(snapshots);
        let mut ch = MonitorsChannel::new(platform);
        assert!(ch.poll());
        assert_eq!(ch.monitors(), &[b, a]);
    }

    fn monitor_strategy() -> impl Strategy<Value = MonitorBounds> {
        (
            -2000.0f32..2000.0,
            -2000.0f32..2000.0,
            1.0f32..1500.0,
            1.0f32..1500.0,
        )
            .prop_map(|(x, y, w, h)| m(x, y, w, h))
    }

    fn snapshot_strategy() -> impl Strategy<Value = Vec<MonitorBounds>> {
        proptest::collection::vec(monitor_strategy(), 0..4)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_new_mirrors_initial_snapshot(initial in snapshot_strategy()) {
            let ch = build(vec![initial.clone()]);
            prop_assert_eq!(ch.monitors(), initial.as_slice());
        }

        #[test]
        fn prop_repeating_same_snapshot_never_changes(
            snap in snapshot_strategy(),
            n in 1usize..6,
        ) {
            let mut snapshots = vec![snap.clone()];
            for _ in 0..n { snapshots.push(snap.clone()); }
            let platform = ScriptedPlatform::new(snapshots);
            let mut ch = MonitorsChannel::new(platform);
            for i in 0..n {
                prop_assert!(!ch.poll(), "poll #{i} unchanged");
                prop_assert_eq!(ch.monitors(), snap.as_slice());
            }
        }

        #[test]
        fn prop_empty_fresh_never_clears_or_changes(
            initial in snapshot_strategy(),
            n_empties in 1usize..6,
        ) {
            prop_assume!(!initial.is_empty());
            let mut snapshots = vec![initial.clone()];
            for _ in 0..n_empties { snapshots.push(Vec::new()); }
            let platform = ScriptedPlatform::new(snapshots);
            let mut ch = MonitorsChannel::new(platform);
            for i in 0..n_empties {
                prop_assert!(!ch.poll(), "empty poll #{i} must be unchanged");
                prop_assert_eq!(ch.monitors(), initial.as_slice(), "step {} retained", i);
            }
        }

        #[test]
        fn prop_distinct_non_empty_triggers_change(
            initial in snapshot_strategy(),
            extra in monitor_strategy(),
        ) {
            let mut next = initial.clone();
            next.push(extra);
            let snapshots = vec![initial.clone(), next.clone()];
            let platform = ScriptedPlatform::new(snapshots);
            let mut ch = MonitorsChannel::new(platform);
            prop_assert!(ch.poll(), "adding a monitor must report change");
            prop_assert_eq!(ch.monitors(), next.as_slice());
        }

        #[test]
        fn prop_change_iff_non_empty_and_different(
            a in snapshot_strategy(),
            b in snapshot_strategy(),
        ) {
            let snapshots = vec![a.clone(), b.clone()];
            let platform = ScriptedPlatform::new(snapshots);
            let mut ch = MonitorsChannel::new(platform);
            let changed = ch.poll();
            let expected = !b.is_empty() && b != a;
            prop_assert_eq!(changed, expected);
            let expected_state = if expected { b } else { a };
            prop_assert_eq!(ch.monitors(), expected_state.as_slice());
        }

        #[test]
        fn prop_min_interval_constant_regardless_of_state(seq in proptest::collection::vec(snapshot_strategy(), 1..8)) {
            let platform = ScriptedPlatform::new(seq.clone());
            let mut ch = MonitorsChannel::new(platform);
            for _ in 1..seq.len() {
                ch.poll();
                prop_assert_eq!(ch.min_interval(), Duration::from_secs(5));
            }
        }
    }
}
