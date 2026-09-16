use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub mod platform;

pub const RECLAIM_DEBOUNCE: Duration = Duration::from_secs(1);

#[derive(Debug, Default)]
pub struct PlaybackStarts {
    playing: HashMap<String, HashSet<u32>>,
    last_reclaim: HashMap<String, Instant>,
    seeded: bool,
}

impl PlaybackStarts {
    pub fn observe(&mut self, now: Instant, playing: &[(String, u32)]) -> Vec<String> {
        let mut current = HashMap::<String, HashSet<u32>>::new();
        for (output, id) in playing {
            current.entry(output.clone()).or_default().insert(*id);
        }
        if !self.seeded {
            self.seeded = true;
            self.playing = current;
            return Vec::new();
        }
        let mut reclaimed = Vec::new();
        for (output, streams) in &current {
            let gained = streams.iter().any(|stream| {
                self.playing
                    .get(output)
                    .is_none_or(|previous| !previous.contains(stream))
            });
            if !gained {
                continue;
            }
            let debounced = self
                .last_reclaim
                .get(output)
                .is_some_and(|last| now.saturating_duration_since(*last) < RECLAIM_DEBOUNCE);
            if debounced {
                continue;
            }
            reclaimed.push(output.clone());
        }
        for output in &reclaimed {
            self.last_reclaim.insert(output.clone(), now);
        }
        self.playing = current;
        reclaimed
    }
}

#[cfg(test)]
mod tests {
    use super::{PlaybackStarts, RECLAIM_DEBOUNCE};
    use std::time::{Duration, Instant};

    fn streams(entries: &[(&str, u32)]) -> Vec<(String, u32)> {
        entries
            .iter()
            .map(|(output, stream)| ((*output).to_string(), *stream))
            .collect()
    }

    #[test]
    fn the_first_snapshot_seeds_without_reclaiming() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        let reclaimed = state.observe(now, &streams(&[("bluez_output.A", 1)]));
        assert!(reclaimed.is_empty());
    }

    #[test]
    fn a_new_stream_reclaims_its_output() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        state.observe(now, &streams(&[]));
        let playing = streams(&[("bluez_output.A", 1)]);
        let reclaimed = state.observe(now + Duration::from_secs(2), &playing);
        assert_eq!(reclaimed, vec!["bluez_output.A".to_string()]);
    }

    #[test]
    fn an_unchanged_snapshot_reclaims_nothing() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        let playing = streams(&[("bluez_output.A", 1)]);
        state.observe(now, &playing);
        let reclaimed = state.observe(now + Duration::from_secs(5), &playing);
        assert!(reclaimed.is_empty());
    }

    #[test]
    fn a_stream_that_returns_reclaims_again_past_the_debounce() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        state.observe(now, &streams(&[]));
        let playing = streams(&[("bluez_output.A", 1)]);
        let first = state.observe(now, &playing);
        assert_eq!(first, vec!["bluez_output.A".to_string()]);
        state.observe(now + Duration::from_secs(1), &streams(&[]));
        let second = state.observe(now + RECLAIM_DEBOUNCE + Duration::from_secs(1), &playing);
        assert_eq!(second, vec!["bluez_output.A".to_string()]);
    }

    #[test]
    fn a_second_new_stream_inside_the_debounce_reclaims_nothing() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        state.observe(now, &streams(&[]));
        let first = state.observe(now, &streams(&[("bluez_output.A", 1)]));
        assert_eq!(first, vec!["bluez_output.A".to_string()]);
        let busy = streams(&[("bluez_output.A", 1), ("bluez_output.A", 2)]);
        let reclaimed = state.observe(now + RECLAIM_DEBOUNCE - Duration::from_millis(1), &busy);
        assert!(reclaimed.is_empty());
    }
}
