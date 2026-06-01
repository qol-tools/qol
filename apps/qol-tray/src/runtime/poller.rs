use std::time::Duration;

pub(crate) trait PollStrategy: Send {
    fn next_interval(
        &mut self,
        current: Duration,
        changed: bool,
        min: Duration,
        max: Duration,
    ) -> Duration;
}

pub(crate) struct BasicStrategy;

impl PollStrategy for BasicStrategy {
    fn next_interval(
        &mut self,
        current: Duration,
        changed: bool,
        min: Duration,
        max: Duration,
    ) -> Duration {
        let next = if changed {
            current / 2
        } else {
            current.saturating_mul(2)
        };
        next.clamp(min, max)
    }
}

pub(crate) struct AdaptivePoller {
    current: Duration,
    min: Duration,
    max: Duration,
    strategy: Box<dyn PollStrategy>,
}

impl AdaptivePoller {
    pub(crate) fn new(min: Duration, max: Duration, strategy: Box<dyn PollStrategy>) -> Self {
        Self {
            current: max,
            min,
            max,
            strategy,
        }
    }

    pub(crate) fn tick(&mut self, changed: bool) -> Duration {
        self.current = self
            .strategy
            .next_interval(self.current, changed, self.min, self.max);
        self.current
    }

    pub(crate) fn current(&self) -> Duration {
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    type StrategyCase = (&'static str, Duration, bool, Duration, Duration, Duration);

    fn us(n: u64) -> Duration {
        Duration::from_micros(n)
    }

    #[test]
    fn basic_strategy_next_interval_table() {
        let cases: &[StrategyCase] = &[
            (
                "changed halves even ms",
                ms(100),
                true,
                ms(16),
                ms(500),
                ms(50),
            ),
            (
                "changed clamps to min when half below min",
                ms(20),
                true,
                ms(16),
                ms(500),
                ms(16),
            ),
            (
                "changed at min stays at min",
                ms(16),
                true,
                ms(16),
                ms(500),
                ms(16),
            ),
            (
                "unchanged doubles",
                ms(100),
                false,
                ms(16),
                ms(500),
                ms(200),
            ),
            (
                "unchanged clamps to max when double above max",
                ms(400),
                false,
                ms(16),
                ms(500),
                ms(500),
            ),
            (
                "unchanged at max stays at max",
                ms(500),
                false,
                ms(16),
                ms(500),
                ms(500),
            ),
            (
                "changed below min clamps up to min",
                ms(1),
                true,
                ms(16),
                ms(500),
                ms(16),
            ),
            (
                "unchanged above max clamps down to max",
                ms(9999),
                false,
                ms(16),
                ms(500),
                ms(500),
            ),
            (
                "zero current changed clamps to min",
                ms(0),
                true,
                ms(16),
                ms(500),
                ms(16),
            ),
            (
                "zero current unchanged clamps to min (0*2=0 < min)",
                ms(0),
                false,
                ms(16),
                ms(500),
                ms(16),
            ),
            (
                "changed odd ms divides into half-ms (sub-ms precision)",
                ms(33),
                true,
                ms(16),
                ms(500),
                us(16_500),
            ),
            (
                "changed even-multiple-of-min halves cleanly",
                ms(64),
                true,
                ms(16),
                ms(500),
                ms(32),
            ),
            (
                "min equals max forces output to that value (changed)",
                ms(200),
                true,
                ms(250),
                ms(250),
                ms(250),
            ),
            (
                "min equals max forces output to that value (unchanged)",
                ms(200),
                false,
                ms(250),
                ms(250),
                ms(250),
            ),
            (
                "changed clamps to min when current=min (cannot go below)",
                ms(50),
                true,
                ms(50),
                ms(50),
                ms(50),
            ),
        ];

        for (label, current, changed, min, max, expected) in cases {
            let mut strategy = BasicStrategy;
            let got = strategy.next_interval(*current, *changed, *min, *max);
            assert_eq!(got, *expected, "case: {label}");
        }
    }

    #[test]
    fn basic_strategy_unchanged_does_not_panic_on_huge_values() {
        let huge = Duration::from_secs(u64::MAX / 2);
        let max = Duration::from_secs(u64::MAX);
        let mut strategy = BasicStrategy;
        let got = strategy.next_interval(huge, false, Duration::ZERO, max);
        assert!(
            got <= max,
            "saturating_mul must not panic, result {got:?} must be within max"
        );
        assert!(got >= huge, "doubling shrinking is not allowed");
    }

    #[test]
    fn adaptive_poller_starts_at_max() {
        let p = AdaptivePoller::new(ms(16), ms(500), Box::new(BasicStrategy));
        assert_eq!(p.current(), ms(500));
    }

    #[test]
    fn adaptive_poller_changed_decreases_until_min() {
        let mut p = AdaptivePoller::new(ms(16), ms(500), Box::new(BasicStrategy));
        let mut prev = p.current();
        for _ in 0..16 {
            let next = p.tick(true);
            assert!(
                next <= prev,
                "changed must not increase: {next:?} > {prev:?}"
            );
            prev = next;
        }
        assert_eq!(
            p.current(),
            ms(16),
            "must converge to min after enough hits"
        );
    }

    #[test]
    fn adaptive_poller_unchanged_increases_until_max() {
        let mut p = AdaptivePoller::new(ms(16), ms(500), Box::new(BasicStrategy));
        for _ in 0..8 {
            p.tick(true);
        }
        assert_eq!(p.current(), ms(16));
        let mut prev = p.current();
        for _ in 0..16 {
            let next = p.tick(false);
            assert!(
                next >= prev,
                "unchanged must not decrease: {next:?} < {prev:?}"
            );
            prev = next;
        }
        assert_eq!(
            p.current(),
            ms(500),
            "must converge to max after enough misses"
        );
    }

    #[test]
    fn adaptive_poller_tick_returns_same_as_current() {
        let mut p = AdaptivePoller::new(ms(16), ms(500), Box::new(BasicStrategy));
        let cases = [true, false, true, true, false];
        for changed in cases {
            let returned = p.tick(changed);
            assert_eq!(returned, p.current(), "tick must return new current");
        }
    }

    #[test]
    fn adaptive_poller_uses_provided_strategy() {
        struct ConstStrategy(Duration);
        impl PollStrategy for ConstStrategy {
            fn next_interval(
                &mut self,
                _current: Duration,
                _changed: bool,
                _min: Duration,
                _max: Duration,
            ) -> Duration {
                self.0
            }
        }
        let mut p = AdaptivePoller::new(ms(16), ms(500), Box::new(ConstStrategy(ms(123))));
        assert_eq!(p.tick(true), ms(123));
        assert_eq!(p.tick(false), ms(123));
        assert_eq!(p.current(), ms(123));
    }

    #[test]
    fn adaptive_poller_passes_current_changed_min_max_to_strategy() {
        use std::sync::{Arc, Mutex};

        type Call = (Duration, bool, Duration, Duration);
        struct CapturingStrategy {
            seen: Arc<Mutex<Vec<Call>>>,
        }
        impl PollStrategy for CapturingStrategy {
            fn next_interval(
                &mut self,
                current: Duration,
                changed: bool,
                min: Duration,
                max: Duration,
            ) -> Duration {
                self.seen.lock().unwrap().push((current, changed, min, max));
                current
            }
        }

        let seen = Arc::new(Mutex::new(Vec::<Call>::new()));
        let mut p = AdaptivePoller::new(
            ms(7),
            ms(99),
            Box::new(CapturingStrategy {
                seen: Arc::clone(&seen),
            }),
        );
        p.tick(true);
        p.tick(false);
        let captured = seen.lock().unwrap().clone();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0], (ms(99), true, ms(7), ms(99)));
        assert_eq!(captured[1], (ms(99), false, ms(7), ms(99)));
    }

    fn duration_ms_strategy() -> impl Strategy<Value = u64> {
        0u64..10_000
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_basic_strategy_output_within_min_max(
            current in duration_ms_strategy(),
            changed in any::<bool>(),
            min in 0u64..1000,
            extra in 0u64..2000,
        ) {
            let max = min + extra;
            let mut s = BasicStrategy;
            let got = s.next_interval(ms(current), changed, ms(min), ms(max));
            prop_assert!(got >= ms(min), "{got:?} < min {min}");
            prop_assert!(got <= ms(max), "{got:?} > max {max}");
        }

        #[test]
        fn prop_basic_strategy_changed_halves_when_in_range(
            half_target in 16u64..2000,
            min in 0u64..16,
            max_extra in 0u64..2000,
        ) {
            let current = half_target * 2;
            let max = current + max_extra;
            let mut s = BasicStrategy;
            let got = s.next_interval(ms(current), true, ms(min), ms(max));
            prop_assert_eq!(got, ms(half_target));
        }

        #[test]
        fn prop_basic_strategy_unchanged_doubles_when_in_range(
            base in 1u64..1000,
            min in 0u64..1,
            extra in 0u64..2000,
        ) {
            let max = base * 2 + extra;
            let mut s = BasicStrategy;
            let got = s.next_interval(ms(base), false, ms(min), ms(max));
            prop_assert_eq!(got, ms(base * 2));
        }

        #[test]
        fn prop_basic_strategy_min_eq_max_forces_constant(
            current in duration_ms_strategy(),
            changed in any::<bool>(),
            pinned in 0u64..1000,
        ) {
            let mut s = BasicStrategy;
            let got = s.next_interval(ms(current), changed, ms(pinned), ms(pinned));
            prop_assert_eq!(got, ms(pinned));
        }

        #[test]
        fn prop_adaptive_poller_current_always_within_bounds(
            min in 0u64..200,
            extra in 1u64..1000,
            seq in proptest::collection::vec(any::<bool>(), 0..32),
        ) {
            let max = min + extra;
            let mut p = AdaptivePoller::new(ms(min), ms(max), Box::new(BasicStrategy));
            prop_assert!(p.current() >= ms(min));
            prop_assert!(p.current() <= ms(max));
            for changed in seq {
                let cur = p.tick(changed);
                prop_assert_eq!(cur, p.current());
                prop_assert!(p.current() >= ms(min));
                prop_assert!(p.current() <= ms(max));
            }
        }

        #[test]
        fn prop_adaptive_poller_starts_at_max(
            min in 0u64..200,
            extra in 0u64..1000,
        ) {
            let max = min + extra;
            let p = AdaptivePoller::new(ms(min), ms(max), Box::new(BasicStrategy));
            prop_assert_eq!(p.current(), ms(max));
        }

        #[test]
        fn prop_adaptive_poller_all_changed_converges_to_min(
            min in 1u64..50,
            extra in 1u64..1000,
        ) {
            let max = min + extra;
            let mut p = AdaptivePoller::new(ms(min), ms(max), Box::new(BasicStrategy));
            for _ in 0..40 {
                p.tick(true);
            }
            prop_assert_eq!(p.current(), ms(min));
        }

        #[test]
        fn prop_adaptive_poller_all_unchanged_converges_to_max(
            min in 1u64..50,
            extra in 1u64..1000,
        ) {
            let max = min + extra;
            let mut p = AdaptivePoller::new(ms(min), ms(max), Box::new(BasicStrategy));
            for _ in 0..6 {
                p.tick(true);
            }
            for _ in 0..40 {
                p.tick(false);
            }
            prop_assert_eq!(p.current(), ms(max));
        }
    }
}
