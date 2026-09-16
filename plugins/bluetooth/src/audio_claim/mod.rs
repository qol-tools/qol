use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

pub mod platform;

pub const RECLAIM_SETTLE: Duration = Duration::from_secs(2);
pub const RECLAIM_COOLDOWN: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
pub struct PlaybackStarts {
    playing: HashMap<String, HashSet<u32>>,
    pending: HashMap<String, Instant>,
    last_reclaim: HashMap<String, Instant>,
    seeded: bool,
}

impl PlaybackStarts {
    pub fn observe(&mut self, now: Instant, playing: &[(String, u32)]) {
        let mut current = HashMap::<String, HashSet<u32>>::new();
        for (output, id) in playing {
            current.entry(output.clone()).or_default().insert(*id);
        }
        if !self.seeded {
            self.seeded = true;
            self.playing = current;
            return;
        }
        self.pending
            .retain(|output, _| current.contains_key(output));
        for (output, streams) in &current {
            let gained = streams.iter().any(|stream| {
                self.playing
                    .get(output)
                    .is_none_or(|previous| !previous.contains(stream))
            });
            if !gained {
                continue;
            }
            let cooling = self
                .last_reclaim
                .get(output)
                .is_some_and(|last| now.saturating_duration_since(*last) < RECLAIM_COOLDOWN);
            if cooling {
                continue;
            }
            self.pending.entry(output.clone()).or_insert(now);
        }
        self.playing = current;
    }

    pub fn media_started(&mut self, now: Instant) {
        if !self.seeded {
            return;
        }
        for output in self.playing.keys() {
            self.pending.entry(output.clone()).or_insert(now);
        }
    }

    pub fn due(&mut self, now: Instant, running: &HashSet<String>) -> Vec<String> {
        let mut due: Vec<String> = self
            .pending
            .iter()
            .filter(|(output, since)| {
                running.contains(output.as_str())
                    && now.saturating_duration_since(**since) >= RECLAIM_SETTLE
            })
            .map(|(output, _)| output.clone())
            .collect();
        due.sort();
        for output in &due {
            self.pending.remove(output);
            self.last_reclaim.insert(output.clone(), now);
        }
        due
    }

    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending
            .values()
            .min()
            .map(|since| *since + RECLAIM_SETTLE)
    }
}

#[cfg(test)]
mod tests {
    use super::{PlaybackStarts, RECLAIM_COOLDOWN, RECLAIM_SETTLE};
    use std::collections::HashSet;
    use std::time::{Duration, Instant};

    fn streams(entries: &[(&str, u32)]) -> Vec<(String, u32)> {
        entries
            .iter()
            .map(|(output, stream)| ((*output).to_string(), *stream))
            .collect()
    }

    fn running(outputs: &[&str]) -> HashSet<String> {
        outputs.iter().map(|output| (*output).to_string()).collect()
    }

    #[test]
    fn the_first_snapshot_seeds_without_pending() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        state.observe(now, &streams(&[("bluez_output.A", 1)]));
        assert!(state.next_deadline().is_none());
        assert!(state
            .due(now + Duration::from_secs(60), &running(&["bluez_output.A"]))
            .is_empty());
    }

    #[test]
    fn a_new_stream_is_not_due_before_the_settle() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        state.observe(now, &streams(&[]));
        state.observe(now, &streams(&[("bluez_output.A", 1)]));
        assert_eq!(state.next_deadline(), Some(now + RECLAIM_SETTLE));
        let early = now + RECLAIM_SETTLE - Duration::from_millis(1);
        assert!(state.due(early, &running(&["bluez_output.A"])).is_empty());
    }

    #[test]
    fn a_new_stream_is_due_after_the_settle_when_running() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        state.observe(now, &streams(&[]));
        state.observe(now, &streams(&[("bluez_output.A", 1)]));
        assert_eq!(
            state.due(now + RECLAIM_SETTLE, &running(&["bluez_output.A"])),
            vec!["bluez_output.A".to_string()]
        );
        assert!(state.next_deadline().is_none());
    }

    #[test]
    fn a_pending_output_waits_for_running() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        state.observe(now, &streams(&[]));
        state.observe(now, &streams(&[("bluez_output.A", 1)]));
        assert!(state.due(now + RECLAIM_SETTLE, &running(&[])).is_empty());
        assert_eq!(
            state.due(
                now + RECLAIM_SETTLE + Duration::from_secs(1),
                &running(&["bluez_output.A"])
            ),
            vec!["bluez_output.A".to_string()]
        );
    }

    #[test]
    fn a_stream_that_disappears_before_the_settle_clears_pending() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        state.observe(now, &streams(&[]));
        state.observe(now, &streams(&[("bluez_output.A", 1)]));
        state.observe(now + Duration::from_secs(1), &streams(&[]));
        assert!(state.next_deadline().is_none());
        assert!(state
            .due(now + RECLAIM_SETTLE, &running(&["bluez_output.A"]))
            .is_empty());
    }

    #[test]
    fn changed_stream_ids_inside_the_cooldown_never_become_due() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        state.observe(now, &streams(&[]));
        state.observe(now, &streams(&[("bluez_output.A", 1)]));
        let reclaimed = state.due(now + RECLAIM_SETTLE, &running(&["bluez_output.A"]));
        assert_eq!(reclaimed, vec!["bluez_output.A".to_string()]);
        state.observe(
            now + RECLAIM_SETTLE + Duration::from_millis(500),
            &streams(&[]),
        );
        state.observe(
            now + RECLAIM_SETTLE + Duration::from_secs(1),
            &streams(&[("bluez_output.A", 2)]),
        );
        assert!(state.next_deadline().is_none());
        assert!(state
            .due(
                now + RECLAIM_SETTLE + RECLAIM_COOLDOWN + Duration::from_secs(1),
                &running(&["bluez_output.A"])
            )
            .is_empty());
    }

    #[test]
    fn a_new_stream_after_the_cooldown_becomes_due_again() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        state.observe(now, &streams(&[]));
        state.observe(now, &streams(&[("bluez_output.A", 1)]));
        let reclaimed = state.due(now + RECLAIM_SETTLE, &running(&["bluez_output.A"]));
        assert_eq!(reclaimed, vec!["bluez_output.A".to_string()]);
        state.observe(now + RECLAIM_SETTLE + Duration::from_secs(1), &streams(&[]));
        let later = now + RECLAIM_SETTLE + RECLAIM_COOLDOWN;
        state.observe(later, &streams(&[("bluez_output.A", 2)]));
        assert_eq!(state.next_deadline(), Some(later + RECLAIM_SETTLE));
        assert_eq!(
            state.due(later + RECLAIM_SETTLE, &running(&["bluez_output.A"])),
            vec!["bluez_output.A".to_string()]
        );
    }

    #[test]
    fn an_unchanged_snapshot_never_becomes_pending() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        let playing = streams(&[("bluez_output.A", 1)]);
        state.observe(now, &playing);
        state.observe(now + Duration::from_secs(5), &playing);
        assert!(state.next_deadline().is_none());
        assert!(state
            .due(now + Duration::from_secs(60), &running(&["bluez_output.A"]))
            .is_empty());
    }

    #[test]
    fn media_started_marks_outputs_with_streams_pending() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        state.observe(now, &streams(&[("bluez_output.A", 1)]));
        state.media_started(now);
        assert_eq!(state.next_deadline(), Some(now + RECLAIM_SETTLE));
        assert_eq!(
            state.due(now + RECLAIM_SETTLE, &running(&["bluez_output.A"])),
            vec!["bluez_output.A".to_string()]
        );
    }

    #[test]
    fn media_started_ignores_outputs_without_streams() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        state.observe(now, &streams(&[("bluez_output.A", 1)]));
        state.observe(now + Duration::from_secs(1), &streams(&[]));
        state.media_started(now + Duration::from_secs(1));
        assert!(state.next_deadline().is_none());
        assert!(state
            .due(now + Duration::from_secs(60), &running(&["bluez_output.A"]))
            .is_empty());
    }

    #[test]
    fn media_started_bypasses_the_cooldown() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        state.observe(now, &streams(&[]));
        state.observe(now, &streams(&[("bluez_output.A", 1)]));
        assert_eq!(
            state.due(now + RECLAIM_SETTLE, &running(&["bluez_output.A"])),
            vec!["bluez_output.A".to_string()]
        );
        let played_again = now + RECLAIM_SETTLE + Duration::from_secs(1);
        state.media_started(played_again);
        assert_eq!(state.next_deadline(), Some(played_again + RECLAIM_SETTLE));
        assert_eq!(
            state.due(played_again + RECLAIM_SETTLE, &running(&["bluez_output.A"])),
            vec!["bluez_output.A".to_string()]
        );
    }

    #[test]
    fn media_started_does_nothing_before_the_first_snapshot() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        state.media_started(now);
        assert!(state.next_deadline().is_none());
        state.observe(now, &streams(&[("bluez_output.A", 1)]));
        assert!(state.next_deadline().is_none());
    }

    #[test]
    fn next_deadline_tracks_the_earliest_pending_output() {
        let now = Instant::now();
        let mut state = PlaybackStarts::default();
        state.observe(now, &streams(&[]));
        state.observe(now, &streams(&[("bluez_output.A", 1)]));
        state.observe(
            now + Duration::from_secs(3),
            &streams(&[("bluez_output.A", 1), ("bluez_output.B", 2)]),
        );
        assert_eq!(state.next_deadline(), Some(now + RECLAIM_SETTLE));
        assert_eq!(
            state.due(now + RECLAIM_SETTLE, &running(&["bluez_output.A"])),
            vec!["bluez_output.A".to_string()]
        );
        assert_eq!(
            state.next_deadline(),
            Some(now + Duration::from_secs(3) + RECLAIM_SETTLE)
        );
    }
}
