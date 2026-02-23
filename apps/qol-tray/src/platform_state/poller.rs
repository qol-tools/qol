use std::time::Duration;

pub(crate) trait PollStrategy: Send {
    fn next_interval(&mut self, current: Duration, changed: bool, min: Duration, max: Duration) -> Duration;
}

pub(crate) struct BasicStrategy;

impl PollStrategy for BasicStrategy {
    fn next_interval(&mut self, current: Duration, changed: bool, min: Duration, max: Duration) -> Duration {
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
    fn next_interval(&mut self, current: Duration, changed: bool, min: Duration, max: Duration) -> Duration {
        if changed {
            self.streak = if self.streak < 0 { 1 } else { self.streak.saturating_add(1) };
        } else {
            self.streak = if self.streak > 0 { -1 } else { self.streak.saturating_sub(1) };
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
        self.current = self.strategy.next_interval(self.current, changed, self.min, self.max);
        self.current
    }

    pub(crate) fn current(&self) -> Duration {
        self.current
    }
}
