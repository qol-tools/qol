use std::time::Duration;

use qol_runtime::MonitorBounds;

use super::super::Channel;
use crate::desktop_state::SharedPlatform;

const MIN_INTERVAL: Duration = Duration::from_millis(100);

pub(crate) struct FocusChannel {
    platform: SharedPlatform,
    bounds: Option<MonitorBounds>,
    poll_allowed: bool,
}

impl FocusChannel {
    pub(crate) fn new(platform: SharedPlatform) -> Self {
        let poll_allowed = platform.poll_focused_window();
        Self {
            platform,
            bounds: None,
            poll_allowed,
        }
    }

    pub(crate) fn bounds(&self) -> Option<MonitorBounds> {
        self.bounds
    }
}

impl Channel for FocusChannel {
    fn poll(&mut self) -> bool {
        if !self.poll_allowed {
            return false;
        }
        let fresh = self.platform.focused_window_bounds();
        if fresh.is_some() && fresh != self.bounds {
            log::debug!(
                "[runtime/focus_ch] CHANGED old={:?} new={:?}",
                self.bounds.map(|b| (b.x, b.y, b.width, b.height)),
                fresh.map(|b| (b.x, b.y, b.width, b.height))
            );
            self.bounds = fresh;
            return true;
        }
        false
    }

    fn min_interval(&self) -> Duration {
        MIN_INTERVAL
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use proptest::prelude::*;

    use super::super::super::Channel;
    use super::*;
    use crate::desktop_state::Platform;

    struct ScriptedPlatform {
        bounds: Mutex<Vec<Option<MonitorBounds>>>,
        poll_allowed: bool,
    }

    impl ScriptedPlatform {
        fn new(bounds: Vec<Option<MonitorBounds>>, poll_allowed: bool) -> Arc<Self> {
            Arc::new(Self {
                bounds: Mutex::new(bounds),
                poll_allowed,
            })
        }
    }

    impl Platform for ScriptedPlatform {
        fn cursor_position(&self) -> Option<(f32, f32)> {
            None
        }
        fn focused_window_bounds(&self) -> Option<MonitorBounds> {
            let mut q = self.bounds.lock().unwrap();
            if q.is_empty() {
                return None;
            }
            q.remove(0)
        }
        fn physical_monitors(&self) -> Vec<MonitorBounds> {
            Vec::new()
        }
        fn poll_focused_window(&self) -> bool {
            self.poll_allowed
        }
    }

    fn b(x: f32, y: f32, w: f32, h: f32) -> MonitorBounds {
        MonitorBounds {
            x,
            y,
            width: w,
            height: h,
        }
    }

    fn run(
        bounds: Vec<Option<MonitorBounds>>,
        poll_allowed: bool,
    ) -> (Vec<bool>, Vec<Option<MonitorBounds>>) {
        let platform = ScriptedPlatform::new(bounds.clone(), poll_allowed);
        let mut ch = FocusChannel::new(platform);
        let mut changed = Vec::with_capacity(bounds.len());
        let mut reported = Vec::with_capacity(bounds.len());
        for _ in 0..bounds.len() {
            changed.push(ch.poll());
            reported.push(ch.bounds());
        }
        (changed, reported)
    }

    #[test]
    fn min_interval_is_100ms() {
        let ch = FocusChannel::new(ScriptedPlatform::new(Vec::new(), true));
        assert_eq!(ch.min_interval(), Duration::from_millis(100));
    }

    #[test]
    fn new_starts_with_no_bounds() {
        let ch = FocusChannel::new(ScriptedPlatform::new(Vec::new(), true));
        assert_eq!(ch.bounds(), None);
    }

    #[test]
    fn poll_disallowed_never_changes_state() {
        let bounds = vec![Some(b(0.0, 0.0, 100.0, 100.0)); 5];
        let (changed, reported) = run(bounds, false);
        assert_eq!(changed, vec![false; 5]);
        assert_eq!(reported, vec![None; 5]);
    }

    type PollCase = (
        &'static str,
        Vec<Option<MonitorBounds>>,
        Vec<bool>,
        Vec<Option<MonitorBounds>>,
    );

    #[test]
    fn poll_transitions_table() {
        let m1 = b(0.0, 0.0, 800.0, 600.0);
        let m2 = b(800.0, 0.0, 1024.0, 768.0);
        let cases: &[PollCase] = &[
            (
                "all none stays none",
                vec![None, None, None],
                vec![false; 3],
                vec![None; 3],
            ),
            (
                "first some triggers change",
                vec![None, Some(m1)],
                vec![false, true],
                vec![None, Some(m1)],
            ),
            (
                "same bounds repeated only first changes",
                vec![Some(m1), Some(m1), Some(m1)],
                vec![true, false, false],
                vec![Some(m1), Some(m1), Some(m1)],
            ),
            (
                "switching bounds changes again",
                vec![Some(m1), Some(m2)],
                vec![true, true],
                vec![Some(m1), Some(m2)],
            ),
            (
                "none after some keeps last bounds (no clear)",
                vec![Some(m1), None, None],
                vec![true, false, false],
                vec![Some(m1), Some(m1), Some(m1)],
            ),
            (
                "none then re-emit same bounds is unchanged",
                vec![Some(m1), None, Some(m1)],
                vec![true, false, false],
                vec![Some(m1), Some(m1), Some(m1)],
            ),
            (
                "none then different bounds is changed",
                vec![Some(m1), None, Some(m2)],
                vec![true, false, true],
                vec![Some(m1), Some(m1), Some(m2)],
            ),
        ];
        for (label, seq, exp_changed, exp_bounds) in cases {
            let (got_changed, got_bounds) = run(seq.clone(), true);
            assert_eq!(&got_changed, exp_changed, "changed mismatch ({label})");
            assert_eq!(&got_bounds, exp_bounds, "bounds mismatch ({label})");
        }
    }

    fn bounds_strategy() -> impl Strategy<Value = MonitorBounds> {
        (
            -2000.0f32..2000.0,
            -2000.0f32..2000.0,
            1.0f32..1500.0,
            1.0f32..1500.0,
        )
            .prop_map(|(x, y, w, h)| b(x, y, w, h))
    }

    fn opt_bounds_strategy() -> impl Strategy<Value = Option<MonitorBounds>> {
        prop_oneof![Just(None), bounds_strategy().prop_map(Some)]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_disallowed_poll_returns_false_and_keeps_none(seq in proptest::collection::vec(opt_bounds_strategy(), 0..16)) {
            let (changed, reported) = run(seq.clone(), false);
            prop_assert!(changed.iter().all(|c| !c));
            prop_assert!(reported.iter().all(|r| r.is_none()));
        }

        #[test]
        fn prop_none_never_clears_bounds(
            initial in bounds_strategy(),
            n_nones in 1usize..10,
        ) {
            let mut seq = vec![Some(initial)];
            seq.extend(std::iter::repeat_n(None, n_nones));
            let (changed, reported) = run(seq, true);
            prop_assert!(changed[0]);
            for (i, c) in changed.iter().enumerate().skip(1) {
                prop_assert!(!c, "None poll #{i} must report unchanged");
            }
            for (i, r) in reported.iter().enumerate().skip(1) {
                prop_assert_eq!(*r, Some(initial), "bounds at step {} must be retained", i);
            }
        }

        #[test]
        fn prop_repeating_same_bounds_changes_once(
            m in bounds_strategy(),
            n in 2usize..10,
        ) {
            let seq = vec![Some(m); n];
            let (changed, _) = run(seq, true);
            prop_assert!(changed[0]);
            for (i, c) in changed.iter().enumerate().skip(1) {
                prop_assert!(!c, "repeat #{i} must be unchanged");
            }
        }

        #[test]
        fn prop_distinct_bounds_always_change(
            m1 in bounds_strategy(),
            m2 in bounds_strategy(),
        ) {
            prop_assume!(m1 != m2);
            let (changed, reported) = run(vec![Some(m1), Some(m2)], true);
            prop_assert_eq!(changed, vec![true, true]);
            prop_assert_eq!(reported, vec![Some(m1), Some(m2)]);
        }

        #[test]
        fn prop_min_interval_constant_regardless_of_state(
            seq in proptest::collection::vec(opt_bounds_strategy(), 0..16),
            poll_allowed in any::<bool>(),
        ) {
            let platform = ScriptedPlatform::new(seq.clone(), poll_allowed);
            let mut ch = FocusChannel::new(platform);
            for _ in 0..seq.len() {
                ch.poll();
                prop_assert_eq!(ch.min_interval(), Duration::from_millis(100));
            }
        }
    }
}
