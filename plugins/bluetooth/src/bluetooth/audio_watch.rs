use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct AudioWatchState {
    attempts: u32,
    last_attempt: Option<Instant>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RepairDecision {
    Repair,
    Cooldown,
    Exhausted,
}

impl AudioWatchState {
    pub fn decide(&self, now: Instant, cooldown: Duration, max_attempts: u32) -> RepairDecision {
        if self.attempts >= max_attempts {
            return RepairDecision::Exhausted;
        }
        if self
            .last_attempt
            .is_some_and(|last| now.saturating_duration_since(last) < cooldown)
        {
            return RepairDecision::Cooldown;
        }
        RepairDecision::Repair
    }

    pub fn attempted(&mut self, now: Instant) {
        self.attempts = self.attempts.saturating_add(1);
        self.last_attempt = Some(now);
    }

    pub fn attempts(&self) -> u32 {
        self.attempts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COOLDOWN: Duration = Duration::from_secs(60);

    #[test]
    fn a_fresh_state_repairs() {
        let state = AudioWatchState::default();
        assert_eq!(
            state.decide(Instant::now(), COOLDOWN, 3),
            RepairDecision::Repair
        );
    }

    #[test]
    fn an_attempt_cooldowns_until_the_window_elapses() {
        let now = Instant::now();
        let mut state = AudioWatchState::default();
        state.attempted(now);
        assert_eq!(state.attempts(), 1);
        assert_eq!(
            state.decide(now + COOLDOWN - Duration::from_millis(1), COOLDOWN, 3),
            RepairDecision::Cooldown
        );
        assert_eq!(
            state.decide(now + COOLDOWN, COOLDOWN, 3),
            RepairDecision::Repair
        );
    }

    #[test]
    fn exhausting_the_attempts_cap_beats_elapsed_time() {
        let now = Instant::now();
        let mut state = AudioWatchState::default();
        for _ in 0..3 {
            state.attempted(now);
        }
        assert_eq!(state.attempts(), 3);
        assert_eq!(
            state.decide(now + COOLDOWN * 10, COOLDOWN, 3),
            RepairDecision::Exhausted
        );
    }

    #[test]
    fn zero_max_attempts_is_exhausted_immediately() {
        let state = AudioWatchState::default();
        assert_eq!(
            state.decide(Instant::now(), COOLDOWN, 0),
            RepairDecision::Exhausted
        );
    }
}
