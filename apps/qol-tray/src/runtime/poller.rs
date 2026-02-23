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
