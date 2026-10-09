use std::collections::VecDeque;
use std::time::Duration;

#[derive(Debug, Clone, Copy)]
pub(super) struct Clock {
    rate: u64,
    emitted: u64,
}

impl Clock {
    pub(super) fn new(rate: u32) -> Self {
        Self {
            rate: u64::from(rate),
            emitted: 0,
        }
    }

    pub(super) fn frames_due(&mut self, elapsed: Duration) -> usize {
        let target = elapsed.as_nanos() * u128::from(self.rate) / 1_000_000_000;
        let target = u64::try_from(target).unwrap_or(u64::MAX);
        let due = target.saturating_sub(self.emitted);
        self.emitted += due;
        usize::try_from(due).unwrap_or(usize::MAX)
    }
}

pub(super) fn push_pcm(buffer: &mut VecDeque<i16>, carry: &mut Option<u8>, bytes: &[u8]) {
    let mut bytes = bytes.iter().copied();
    if let Some(low) = carry.take() {
        match bytes.next() {
            Some(high) => buffer.push_back(i16::from_le_bytes([low, high])),
            None => {
                *carry = Some(low);
                return;
            }
        }
    }
    loop {
        match (bytes.next(), bytes.next()) {
            (Some(low), Some(high)) => buffer.push_back(i16::from_le_bytes([low, high])),
            (Some(low), None) => {
                *carry = Some(low);
                return;
            }
            _ => return,
        }
    }
}

pub(super) fn trim_backlog(buffer: &mut VecDeque<i16>, max_samples: usize, channels: usize) {
    let channels = channels.max(1);
    let excess = buffer.len().saturating_sub(max_samples);
    if excess == 0 {
        return;
    }
    let drop = excess.div_ceil(channels) * channels;
    buffer.drain(..drop.min(buffer.len()));
}

pub(super) fn take_padded(buffer: &mut VecDeque<i16>, samples: usize) -> Vec<i16> {
    let available = buffer.len().min(samples);
    let mut track = buffer.drain(..available).collect::<Vec<_>>();
    track.resize(samples, 0);
    track
}

pub(super) fn mix(tracks: &[Vec<i16>], samples: usize) -> Vec<i16> {
    (0..samples)
        .map(|index| {
            let sum = tracks
                .iter()
                .map(|track| i32::from(track.get(index).copied().unwrap_or(0)))
                .sum::<i32>();
            sum.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16
        })
        .collect()
}

pub(super) fn to_le_bytes(samples: &[i16]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{mix, push_pcm, take_padded, to_le_bytes, trim_backlog, Clock};
    use std::collections::VecDeque;
    use std::time::Duration;

    #[test]
    fn clock_emits_wall_time_worth_of_frames_without_drift() {
        let mut clock = Clock::new(48_000);
        let cases = [(0, 0), (10, 480), (25, 720), (25, 0), (1_000, 46_800)];
        for (elapsed_ms, due) in cases {
            assert_eq!(
                clock.frames_due(Duration::from_millis(elapsed_ms)),
                due,
                "{elapsed_ms} ms"
            );
        }
    }

    #[test]
    fn pcm_bytes_join_across_split_reads() {
        let mut buffer = VecDeque::new();
        let mut carry = None;
        let samples = [1i16, -2, 300, i16::MIN];
        let bytes = to_le_bytes(&samples);
        for chunk in [&bytes[..1], &bytes[1..4], &bytes[4..5], &bytes[5..]] {
            push_pcm(&mut buffer, &mut carry, chunk);
        }
        assert_eq!(buffer.into_iter().collect::<Vec<_>>(), samples);
        assert_eq!(carry, None);
    }

    #[test]
    fn missing_audio_is_padded_with_silence() {
        let cases: [(&[i16], usize, &[i16], usize); 3] = [
            (&[], 4, &[0, 0, 0, 0], 0),
            (&[5, 6], 4, &[5, 6, 0, 0], 0),
            (&[1, 2, 3, 4, 5, 6], 4, &[1, 2, 3, 4], 2),
        ];
        for (queued, samples, expected, left) in cases {
            let mut buffer = queued.iter().copied().collect::<VecDeque<_>>();
            assert_eq!(take_padded(&mut buffer, samples), expected, "{queued:?}");
            assert_eq!(buffer.len(), left, "{queued:?}");
        }
    }

    #[test]
    fn backlog_trims_whole_frames_from_the_front() {
        let cases: [(usize, usize, &[i16]); 3] = [
            (6, 2, &[1, 2, 3, 4, 5, 6]),
            (4, 2, &[3, 4, 5, 6]),
            (3, 2, &[5, 6]),
        ];
        for (max, channels, expected) in cases {
            let mut buffer = VecDeque::from(vec![1, 2, 3, 4, 5, 6]);
            trim_backlog(&mut buffer, max, channels);
            assert_eq!(buffer.into_iter().collect::<Vec<_>>(), expected, "{max}");
        }
    }

    #[test]
    fn tracks_sum_and_saturate() {
        let cases: [(&[Vec<i16>], &[i16]); 3] = [
            (&[], &[0, 0]),
            (&[vec![100, -100], vec![20, 30]], &[120, -70]),
            (
                &[vec![i16::MAX, i16::MIN], vec![10, -10]],
                &[i16::MAX, i16::MIN],
            ),
        ];
        for (tracks, expected) in cases {
            assert_eq!(mix(tracks, 2), expected, "{tracks:?}");
        }
    }
}
