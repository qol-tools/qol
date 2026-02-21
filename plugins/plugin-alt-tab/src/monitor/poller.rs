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

pub(crate) struct MomentumStrategy {
    streak: i32,
}

impl MomentumStrategy {
    pub(crate) fn new() -> Self {
        Self { streak: 0 }
    }
}

impl PollStrategy for MomentumStrategy {
    fn next_interval(
        &mut self,
        current: Duration,
        changed: bool,
        min: Duration,
        max: Duration,
    ) -> Duration {
        if changed {
            self.streak = if self.streak < 0 {
                1
            } else {
                self.streak.saturating_add(1)
            };
        } else {
            self.streak = if self.streak > 0 {
                -1
            } else {
                self.streak.saturating_sub(1)
            };
        }

        let abs = self.streak.unsigned_abs().max(1);

        let next = if changed {
            let divisor = abs.clamp(2, 8) as u32;
            current / divisor
        } else {
            let numer = abs + 1;
            let denom = abs;
            current * numer / denom
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

    pub(crate) fn reconfigure(
        &mut self,
        min: Duration,
        max: Duration,
        strategy: Box<dyn PollStrategy>,
    ) {
        self.min = min;
        self.max = max;
        self.strategy = strategy;
        self.current = self.current.clamp(self.min, self.max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::time::Duration;

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    #[test]
    fn basic_strategy_halves_on_change() {
        let cases = [
            (ms(500), ms(250)),
            (ms(128), ms(64)),
            (ms(32), ms(16)),
            (ms(16), ms(16)),
        ];
        let mut s = BasicStrategy;
        for (current, expected) in cases {
            assert_eq!(
                s.next_interval(current, true, ms(16), ms(500)),
                expected,
                "current: {current:?}"
            );
        }
    }

    #[test]
    fn basic_strategy_doubles_on_idle() {
        let cases = [
            (ms(16), ms(32)),
            (ms(64), ms(128)),
            (ms(256), ms(500)),
            (ms(500), ms(500)),
        ];
        let mut s = BasicStrategy;
        for (current, expected) in cases {
            assert_eq!(
                s.next_interval(current, false, ms(16), ms(500)),
                expected,
                "current: {current:?}"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_interval_stays_in_bounds(
            current_ms in 1u64..2000,
            changed in proptest::bool::ANY,
        ) {
            let min = ms(16);
            let max = ms(500);
            let current = ms(current_ms);
            let mut s = BasicStrategy;
            let next = s.next_interval(current, changed, min, max);
            prop_assert!(next >= min && next <= max,
                "next={next:?} out of bounds [{min:?}, {max:?}]");
        }
    }

    #[test]
    fn poller_ramps_down_then_up() {
        let mut p = AdaptivePoller::new(ms(16), ms(500), Box::new(BasicStrategy));
        assert_eq!(p.current(), ms(500));

        let after_change = p.tick(true);
        assert!(after_change < ms(500), "should ramp down");

        let mut interval = after_change;
        for _ in 0..20 {
            interval = p.tick(false);
        }
        assert_eq!(
            interval,
            ms(500),
            "should reach max after enough idle ticks"
        );
    }

    #[test]
    fn momentum_ramp_down_gets_more_aggressive_with_idle_streak() {
        let mut s = MomentumStrategy::new();
        for _ in 0..10 {
            s.next_interval(ms(500), false, ms(16), ms(500));
        }

        let after_1 = s.next_interval(ms(500), true, ms(16), ms(500));

        let mut s2 = MomentumStrategy::new();
        let after_fresh = s2.next_interval(ms(500), true, ms(16), ms(500));

        assert!(
            after_1 <= after_fresh,
            "long idle streak should ramp down more aggressively: {after_1:?} vs {after_fresh:?}"
        );
    }

    #[test]
    fn momentum_ramp_up_gets_gentler_with_active_streak() {
        let mut s = MomentumStrategy::new();
        for _ in 0..10 {
            s.next_interval(ms(16), true, ms(16), ms(500));
        }
        let after_long_active = s.next_interval(ms(100), false, ms(16), ms(500));

        let mut s2 = MomentumStrategy::new();
        s2.next_interval(ms(100), true, ms(16), ms(500));
        let after_short_active = s2.next_interval(ms(100), false, ms(16), ms(500));

        assert!(after_long_active <= after_short_active,
            "long active streak should ramp up more gently: {after_long_active:?} vs {after_short_active:?}");
    }

    #[test]
    fn momentum_direction_change_resets_streak() {
        let mut s = MomentumStrategy::new();
        for _ in 0..5 {
            s.next_interval(ms(500), false, ms(16), ms(500));
        }
        assert!(s.streak < -1);

        s.next_interval(ms(500), true, ms(16), ms(500));
        assert_eq!(s.streak, 1);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_momentum_stays_in_bounds(
            current_ms in 1u64..2000,
            changed in proptest::bool::ANY,
            prior_ticks in 0u32..50,
        ) {
            let min = ms(16);
            let max = ms(500);
            let mut s = MomentumStrategy::new();
            for _ in 0..prior_ticks {
                s.next_interval(ms(250), !changed, min, max);
            }
            let next = s.next_interval(ms(current_ms), changed, min, max);
            prop_assert!(next >= min && next <= max,
                "next={next:?} out of bounds [{min:?}, {max:?}]");
        }
    }
}
