use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RetryPolicy {
    initial: Duration,
    maximum: Duration,
}

impl RetryPolicy {
    pub fn from_seconds(initial: f64, maximum: f64) -> Self {
        let initial = Duration::from_secs_f64(initial.max(1.0));
        let maximum = Duration::from_secs_f64(maximum.max(initial.as_secs_f64()));
        Self { initial, maximum }
    }

    pub fn delay_after_failure(self, failures: u32) -> Duration {
        let exponent = failures.saturating_sub(1).min(31);
        self.initial
            .saturating_mul(1_u32 << exponent)
            .min(self.maximum)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RetryState {
    failures: u32,
    due: Option<Instant>,
}

impl RetryState {
    pub fn request_now(&mut self, now: Instant) {
        self.due = Some(now);
    }

    pub fn request_when_idle(&mut self, now: Instant) {
        if self.due.is_none() {
            self.due = Some(now);
        }
    }

    pub fn connected(&mut self) {
        self.failures = 0;
        self.due = None;
    }

    pub fn failed(&mut self, now: Instant, policy: RetryPolicy) -> Duration {
        self.failures = self.failures.saturating_add(1);
        let delay = policy.delay_after_failure(self.failures);
        self.due = Some(now + delay);
        delay
    }

    pub fn due(&self) -> Option<Instant> {
        self.due
    }

    pub fn is_due(&self, now: Instant) -> bool {
        self.due.is_some_and(|due| due <= now)
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        let policy = RetryPolicy::from_seconds(2.0, 10.0);
        let cases = [(1, 2), (2, 4), (3, 8), (4, 10), (20, 10)];
        for (failures, expected_seconds) in cases {
            assert_eq!(
                policy.delay_after_failure(failures),
                Duration::from_secs(expected_seconds),
                "failures={failures}"
            );
        }
    }

    #[test]
    fn device_events_wake_an_idle_retry_without_shortening_a_backoff() {
        let now = Instant::now();
        let policy = RetryPolicy::from_seconds(1.0, 60.0);
        let mut state = RetryState::default();
        let delay = state.failed(now, policy);
        state.request_when_idle(now);
        assert_eq!(state.due(), Some(now + delay), "backoff must survive");
        state.connected();
        state.request_when_idle(now);
        assert_eq!(state.due(), Some(now), "idle retry must wake");
    }

    #[test]
    fn successful_connection_resets_pending_retry() {
        let now = Instant::now();
        let policy = RetryPolicy::from_seconds(1.0, 60.0);
        let mut state = RetryState::default();
        state.request_now(now);
        assert!(state.is_due(now));
        state.failed(now, policy);
        assert_eq!(state.failures(), 1);
        state.connected();
        assert_eq!(state.failures(), 0);
        assert_eq!(state.due(), None);
    }
}
