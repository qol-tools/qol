use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const KEEP: Duration = Duration::from_secs(120);
const CAPACITY: usize = 64;

static SHOWN: Mutex<Claims> = Mutex::new(Claims::new());

pub(super) fn record(title: &str, body: &str) {
    lock().record(Instant::now(), title, body);
}

pub(in super::super) fn claimed(title: &str, body: &str) -> bool {
    lock().take(Instant::now(), title, body)
}

fn lock() -> std::sync::MutexGuard<'static, Claims> {
    SHOWN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct Claims {
    entries: VecDeque<(Instant, String, String)>,
}

impl Claims {
    const fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    fn record(&mut self, now: Instant, title: &str, body: &str) {
        self.prune(now);
        if self.entries.len() == CAPACITY {
            self.entries.pop_front();
        }
        self.entries
            .push_back((now, title.to_string(), body.to_string()));
    }

    fn take(&mut self, now: Instant, title: &str, body: &str) -> bool {
        self.prune(now);
        let found = self
            .entries
            .iter()
            .position(|(_, shown_title, shown_body)| shown_title == title && shown_body == body);
        found.and_then(|index| self.entries.remove(index)).is_some()
    }

    fn prune(&mut self, now: Instant) {
        while self
            .entries
            .front()
            .is_some_and(|(at, _, _)| now.duration_since(*at) > KEEP)
        {
            self.entries.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_banner_is_claimed_once() {
        let now = Instant::now();
        let mut claims = Claims::new();
        claims.record(now, "Ada", "hi");
        assert!(!claims.take(now, "Ada", "other"));
        assert!(claims.take(now, "Ada", "hi"));
        assert!(!claims.take(now, "Ada", "hi"));
    }

    #[test]
    fn two_identical_banners_claim_two_records() {
        let now = Instant::now();
        let mut claims = Claims::new();
        claims.record(now, "Ada", "hi");
        claims.record(now, "Ada", "hi");
        assert!(claims.take(now, "Ada", "hi"));
        assert!(claims.take(now, "Ada", "hi"));
        assert!(!claims.take(now, "Ada", "hi"));
    }

    #[test]
    fn old_banners_expire() {
        let now = Instant::now();
        let mut claims = Claims::new();
        claims.record(now, "Ada", "hi");
        assert!(!claims.take(now + KEEP + Duration::from_secs(1), "Ada", "hi"));
    }

    #[test]
    fn capacity_drops_the_oldest() {
        let now = Instant::now();
        let mut claims = Claims::new();
        for index in 0..=CAPACITY {
            claims.record(now, &index.to_string(), "");
        }
        assert!(!claims.take(now, "0", ""));
        assert!(claims.take(now, &CAPACITY.to_string(), ""));
    }
}
