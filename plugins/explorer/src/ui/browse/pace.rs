use std::time::{Duration, Instant};

const GAP: Duration = Duration::from_millis(500);
const BUSY_SHARE: u32 = 5;

#[derive(Debug, Default, Clone, Copy)]
pub(super) struct Pace {
    started: Option<Instant>,
    took: Duration,
}

impl Pace {
    pub(super) fn start(&mut self, now: Instant) {
        self.started = Some(now);
    }

    pub(super) fn finish(&mut self, now: Instant) {
        if let Some(started) = self.started {
            self.took = now.saturating_duration_since(started);
        }
    }

    pub(super) fn started(&self) -> Option<Instant> {
        self.started
    }

    pub(super) fn next(&self, wanted: Instant) -> Instant {
        self.started.map_or(wanted, |started| {
            wanted.max(started + GAP.max(self.took * BUSY_SHARE))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{Pace, GAP};

    #[test]
    fn a_read_waits_for_the_gap_and_a_slow_read_waits_four_times_as_long_as_it_took() {
        let start = Instant::now();
        let mut pace = Pace::default();
        assert_eq!(pace.next(start), start);

        pace.start(start);
        pace.finish(start + Duration::from_millis(10));
        assert_eq!(pace.next(start), start + GAP);
        let late = start + Duration::from_secs(3);
        assert_eq!(pace.next(late), late);

        pace.start(late);
        pace.finish(late + Duration::from_secs(1));
        assert_eq!(pace.next(late), late + Duration::from_secs(5));
    }
}
