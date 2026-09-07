use std::collections::VecDeque;

use crate::audio::{AudioFormat, AudioFrame};

use super::{ListenConfig, UtteranceEndReason};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum SegmentedFrame {
    Idle,
    Started {
        observed_at_ms: u64,
        level_permille: u16,
        frames: Vec<AudioFrame>,
    },
    Active(AudioFrame),
    Ended {
        frame: AudioFrame,
        level_permille: u16,
        reason: UtteranceEndReason,
    },
}

pub(super) struct UtteranceSegmenter {
    detector: UtteranceDetector,
    pre_roll: VecDeque<AudioFrame>,
    pre_roll_samples: u64,
    pre_roll_samples_limit: u64,
    channels: u16,
}

impl UtteranceSegmenter {
    pub(super) fn new(config: ListenConfig, format: AudioFormat) -> Self {
        Self {
            detector: UtteranceDetector::new(config, format.sample_rate),
            pre_roll: VecDeque::new(),
            pre_roll_samples: 0,
            pre_roll_samples_limit: duration_samples(
                format.sample_rate,
                config.pre_roll_ms.max(config.onset_ms),
            ),
            channels: format.channels,
        }
    }

    pub(super) fn observe(&mut self, frame: AudioFrame) -> SegmentedFrame {
        let samples = sample_frames(&frame.pcm, self.channels);
        let signal = signal_level(&frame.pcm);
        match self.detector.observe(signal, samples) {
            DetectorTransition::Idle => {
                self.push_pre_roll(frame, samples);
                SegmentedFrame::Idle
            }
            DetectorTransition::Started { level_permille } => {
                let observed_at_ms = frame.observed_at_ms;
                self.push_pre_roll(frame, samples);
                SegmentedFrame::Started {
                    observed_at_ms,
                    level_permille,
                    frames: self.pre_roll.drain(..).collect(),
                }
            }
            DetectorTransition::Active => SegmentedFrame::Active(frame),
            DetectorTransition::Ended {
                level_permille,
                reason,
            } => SegmentedFrame::Ended {
                frame,
                level_permille,
                reason,
            },
        }
    }

    fn push_pre_roll(&mut self, frame: AudioFrame, samples: u64) {
        self.pre_roll.push_back(frame);
        self.pre_roll_samples = self.pre_roll_samples.saturating_add(samples);
        while self.pre_roll_samples > self.pre_roll_samples_limit {
            let Some(discarded) = self.pre_roll.pop_front() else {
                break;
            };
            self.pre_roll_samples = self
                .pre_roll_samples
                .saturating_sub(sample_frames(&discarded.pcm, self.channels));
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DetectorTransition {
    Idle,
    Started {
        level_permille: u16,
    },
    Active,
    Ended {
        level_permille: u16,
        reason: UtteranceEndReason,
    },
}

struct UtteranceDetector {
    threshold_permille: u16,
    onset_samples_required: u64,
    silence_samples_required: u64,
    max_samples: u64,
    onset_samples: u64,
    silence_samples: u64,
    active_samples: u64,
    active: bool,
}

impl UtteranceDetector {
    fn new(config: ListenConfig, sample_rate: u32) -> Self {
        Self {
            threshold_permille: config.threshold_permille,
            onset_samples_required: duration_samples(sample_rate, config.onset_ms),
            silence_samples_required: duration_samples(sample_rate, config.silence_ms),
            max_samples: duration_samples(sample_rate, config.max_utterance_ms),
            onset_samples: 0,
            silence_samples: 0,
            active_samples: 0,
            active: false,
        }
    }

    fn observe(&mut self, level_permille: u16, samples: u64) -> DetectorTransition {
        if self.active {
            return self.observe_active(level_permille, samples);
        }
        self.observe_idle(level_permille, samples)
    }

    fn observe_idle(&mut self, level_permille: u16, samples: u64) -> DetectorTransition {
        if level_permille < self.threshold_permille {
            self.onset_samples = 0;
            return DetectorTransition::Idle;
        }
        self.onset_samples = self.onset_samples.saturating_add(samples);
        if self.onset_samples < self.onset_samples_required {
            return DetectorTransition::Idle;
        }
        self.active = true;
        self.active_samples = self.onset_samples;
        self.onset_samples = 0;
        DetectorTransition::Started { level_permille }
    }

    fn observe_active(&mut self, level_permille: u16, samples: u64) -> DetectorTransition {
        self.active_samples = self.active_samples.saturating_add(samples);
        if self.active_samples >= self.max_samples {
            return self.end(level_permille, UtteranceEndReason::MaximumDuration);
        }
        if level_permille >= self.threshold_permille {
            self.silence_samples = 0;
            return DetectorTransition::Active;
        }
        self.silence_samples = self.silence_samples.saturating_add(samples);
        if self.silence_samples < self.silence_samples_required {
            return DetectorTransition::Active;
        }
        self.end(level_permille, UtteranceEndReason::Silence)
    }

    fn end(&mut self, level_permille: u16, reason: UtteranceEndReason) -> DetectorTransition {
        self.active = false;
        self.active_samples = 0;
        self.silence_samples = 0;
        DetectorTransition::Ended {
            level_permille,
            reason,
        }
    }
}

fn duration_samples(sample_rate: u32, duration_ms: u64) -> u64 {
    u64::from(sample_rate).saturating_mul(duration_ms) / 1_000
}

fn sample_frames(pcm: &[u8], channels: u16) -> u64 {
    let samples = pcm.len() / 2;
    let channels = usize::from(channels.max(1));
    u64::try_from(samples / channels).unwrap_or(u64::MAX)
}

fn signal_level(pcm: &[u8]) -> u16 {
    let samples = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]));
    let (squared_sum, count) = samples.fold((0.0, 0_usize), |acc, sample| {
        let normalized = f64::from(sample) / 32768.0;
        (acc.0 + normalized * normalized, acc.1 + 1)
    });
    if count == 0 {
        return 0;
    }
    let rms = (squared_sum / count as f64).sqrt();
    let scaled = (rms * 1000.0).round();
    scaled.clamp(0.0, 1000.0) as u16
}

#[cfg(test)]
mod tests {
    use crate::audio::{AudioEncoding, AudioFormat, AudioFrame};

    use super::{ListenConfig, SegmentedFrame, UtteranceEndReason, UtteranceSegmenter};

    #[test]
    fn transient_sound_does_not_open_an_utterance() {
        let mut segmenter = segmenter(config(100, 100, 300, 1_000));

        assert!(matches!(
            segmenter.observe(frame(50, 1_000, 50)),
            SegmentedFrame::Idle
        ));
        assert!(matches!(
            segmenter.observe(frame(100, 0, 50)),
            SegmentedFrame::Idle
        ));
    }

    #[test]
    fn onset_audio_is_retained_when_optional_preroll_is_disabled() {
        let mut segmenter = segmenter(config(100, 100, 0, 1_000));
        segmenter.observe(frame(50, 1_000, 50));

        let SegmentedFrame::Started { frames, .. } = segmenter.observe(frame(100, 1_000, 50))
        else {
            panic!("expected an utterance to start");
        };

        assert_eq!(frames.len(), 2);
    }

    #[test]
    fn onset_replays_preroll_into_the_new_utterance() {
        let mut segmenter = segmenter(config(100, 100, 150, 1_000));
        segmenter.observe(frame(50, 0, 50));
        segmenter.observe(frame(100, 1_000, 50));

        let SegmentedFrame::Started { frames, .. } = segmenter.observe(frame(150, 1_000, 50))
        else {
            panic!("expected an utterance to start");
        };

        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].observed_at_ms, 50);
        assert_eq!(frames[2].observed_at_ms, 150);
    }

    #[test]
    fn silence_and_maximum_duration_have_distinct_end_reasons() {
        let mut silence = segmenter(config(50, 100, 100, 1_000));
        silence.observe(frame(50, 1_000, 50));
        silence.observe(frame(100, 1_000, 50));
        silence.observe(frame(150, 0, 50));
        assert!(matches!(
            silence.observe(frame(200, 0, 50)),
            SegmentedFrame::Ended {
                reason: UtteranceEndReason::Silence,
                ..
            }
        ));

        let mut maximum = segmenter(config(50, 500, 100, 150));
        maximum.observe(frame(50, 1_000, 50));
        maximum.observe(frame(100, 1_000, 50));
        assert!(matches!(
            maximum.observe(frame(150, 1_000, 50)),
            SegmentedFrame::Ended {
                reason: UtteranceEndReason::MaximumDuration,
                ..
            }
        ));
    }

    fn segmenter(config: ListenConfig) -> UtteranceSegmenter {
        UtteranceSegmenter::new(
            config,
            AudioFormat {
                sample_rate: 1_000,
                channels: 1,
                encoding: AudioEncoding::PcmS16Le,
            },
        )
    }

    fn config(
        onset_ms: u64,
        silence_ms: u64,
        pre_roll_ms: u64,
        max_utterance_ms: u64,
    ) -> ListenConfig {
        ListenConfig {
            threshold_permille: 10,
            onset_ms,
            silence_ms,
            pre_roll_ms,
            max_utterance_ms,
        }
    }

    fn frame(observed_at_ms: u64, sample: i16, samples: usize) -> AudioFrame {
        AudioFrame {
            observed_at_ms,
            pcm: std::iter::repeat_n(sample.to_le_bytes(), samples)
                .flatten()
                .collect(),
        }
    }
}
