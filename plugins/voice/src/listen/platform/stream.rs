use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crossbeam_channel::Sender;
use qol_audio::default_output;
use qol_audio::devices::Direction;

use crate::audio::{AudioEncoding, AudioFormat, AudioFrame};

use super::super::{ActiveAudioInput, AudioInputProbe, ListenError, ListenMessage};

pub(super) const SAMPLE_RATE: u32 = 16_000;
pub(super) const CHANNELS: u16 = 1;
const BUFFER_BYTES: usize = 3_200;
const CAPTURE_START_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) const FORMAT: AudioFormat = AudioFormat {
    sample_rate: SAMPLE_RATE,
    channels: CHANNELS,
    encoding: AudioEncoding::PcmS16Le,
};

type Stop = Box<dyn FnOnce() + Send>;

pub(super) fn default_input() -> Result<String, ListenError> {
    match default_output::effective(Direction::Input) {
        Ok(Some(name)) if !name.is_empty() => Ok(name),
        Ok(_) => Err(ListenError::NoInputDevice),
        Err(error) => Err(ListenError::InputUnavailable(format!(
            "could not resolve the default audio source: {error}"
        ))),
    }
}

pub(super) struct Frames {
    pub(super) session_started_at: Instant,
    pub(super) frames: SyncSender<AudioFrame>,
    pub(super) dropped: Arc<AtomicU64>,
    pub(super) events: Sender<ListenMessage>,
}

struct StreamInput {
    stop: Mutex<Option<Stop>>,
    stopping: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
}

impl ActiveAudioInput for StreamInput {}

impl Drop for StreamInput {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(stop) = self.stop.get_mut().ok().and_then(Option::take) {
            stop();
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

pub(super) fn start<R>(
    audio: R,
    stop: impl FnOnce() + Send + 'static,
    device_name: &str,
    frames: Frames,
) -> Result<Box<dyn ActiveAudioInput>, ListenError>
where
    R: Read + Send + 'static,
{
    let stopping = Arc::new(AtomicBool::new(false));
    let reader_stopping = stopping.clone();
    let (capture_started, capture_start) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        capture_audio(audio, frames, &reader_stopping, capture_started);
    });
    let input = StreamInput {
        stop: Mutex::new(Some(Box::new(stop))),
        stopping,
        reader: Some(reader),
    };
    wait_for_capture_start(&capture_start, device_name)?;
    Ok(Box::new(input))
}

fn capture_audio<R>(
    mut audio: R,
    sink: Frames,
    stopping: &AtomicBool,
    capture_started: SyncSender<Result<(), String>>,
) where
    R: Read,
{
    let mut buffer = [0_u8; BUFFER_BYTES];
    let mut capture_started = Some(capture_started);
    loop {
        if let Err(error) = audio.read_exact(&mut buffer) {
            let message = error.to_string();
            if let Some(started) = capture_started.take() {
                let _ = started.send(Err(message.clone()));
            }
            if !stopping.load(Ordering::Acquire) {
                let _ = sink.events.send(Err(ListenError::CaptureFailed(message)));
            }
            return;
        }
        if let Some(started) = capture_started.take() {
            if started.send(Ok(())).is_err() {
                return;
            }
        }
        let observed_at_ms =
            u64::try_from(sink.session_started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        let frame = AudioFrame {
            observed_at_ms,
            pcm: buffer.to_vec(),
        };
        match sink.frames.try_send(frame) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                sink.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => return,
        }
    }
}

fn wait_for_capture_start(
    capture_start: &Receiver<Result<(), String>>,
    device_name: &str,
) -> Result<(), ListenError> {
    match capture_start.recv_timeout(CAPTURE_START_TIMEOUT) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(message)) => Err(ListenError::CaptureFailed(message)),
        Err(RecvTimeoutError::Disconnected) => Err(ListenError::CaptureFailed(
            "audio reader stopped before producing a frame".to_owned(),
        )),
        Err(RecvTimeoutError::Timeout) => Err(ListenError::InputUnavailable(format!(
            "source '{device_name}' produced no audio for 3 seconds; choose another input or check its hardware mute"
        ))),
    }
}

pub(super) fn probe<R, S>(
    device_name: String,
    duration_ms: u64,
    open: impl FnOnce(&str) -> Result<(R, S), ListenError>,
) -> Result<AudioInputProbe, ListenError>
where
    R: Read + Send + 'static,
    S: FnOnce(),
{
    if duration_ms == 0 {
        return Err(ListenError::InputUnavailable(
            "probe duration must be greater than zero".to_owned(),
        ));
    }
    let target_bytes = u64::from(SAMPLE_RATE)
        .saturating_mul(2)
        .saturating_mul(duration_ms)
        / 1_000;
    let (audio, stop) = open(&device_name)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut pcm = Vec::new();
        let result = audio.take(target_bytes).read_to_end(&mut pcm).map(|_| pcm);
        let _ = sender.send(result);
    });
    let timeout = Duration::from_millis(duration_ms).saturating_add(CAPTURE_START_TIMEOUT);
    let received = receiver.recv_timeout(timeout);
    stop();
    let _ = reader.join();
    let pcm = match received {
        Ok(Ok(pcm)) if !pcm.is_empty() => pcm,
        Ok(Ok(_)) => {
            return Err(ListenError::CaptureFailed(
                "audio source returned no samples".to_owned(),
            ))
        }
        Ok(Err(error)) => return Err(ListenError::CaptureFailed(error.to_string())),
        Err(RecvTimeoutError::Disconnected) => {
            return Err(ListenError::CaptureFailed(
                "audio probe stopped before producing samples".to_owned(),
            ))
        }
        Err(RecvTimeoutError::Timeout) => {
            return Err(ListenError::InputUnavailable(format!(
                "source '{device_name}' produced no audio during the probe"
            )))
        }
    };
    Ok(probe_report(device_name, &pcm))
}

fn probe_report(device_id: String, pcm: &[u8]) -> AudioInputProbe {
    let samples = pcm
        .as_chunks::<2>()
        .0
        .iter()
        .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
        .collect::<Vec<_>>();
    let count = u64::try_from(samples.len()).unwrap_or(u64::MAX);
    let squared_sum = samples
        .iter()
        .map(|sample| {
            let normalized = f64::from(*sample) / 32768.0;
            normalized * normalized
        })
        .sum::<f64>();
    let rms = if samples.is_empty() {
        0.0
    } else {
        (squared_sum / samples.len() as f64).sqrt()
    };
    let peak = samples
        .iter()
        .map(|sample| sample.unsigned_abs())
        .max()
        .unwrap_or(0);
    let nonzero = samples.iter().filter(|sample| **sample != 0).count();
    let clipped = samples
        .iter()
        .filter(|sample| sample.unsigned_abs() >= 32_760)
        .count();
    AudioInputProbe {
        device_id,
        captured_ms: count.saturating_mul(1_000) / u64::from(SAMPLE_RATE),
        peak_permille: scale_peak(peak),
        rms_permille: scale_level(rms),
        nonzero_permille: scale_ratio(nonzero, samples.len()),
        clipped_samples: u64::try_from(clipped).unwrap_or(u64::MAX),
    }
}

fn scale_peak(peak: u16) -> u16 {
    u16::try_from(u32::from(peak).saturating_mul(1_000) / 32_768).unwrap_or(1_000)
}

fn scale_level(level: f64) -> u16 {
    (level * 1_000.0).round().clamp(0.0, 1_000.0) as u16
}

fn scale_ratio(value: usize, total: usize) -> u16 {
    if total == 0 {
        return 0;
    }
    u16::try_from(value.saturating_mul(1_000) / total).unwrap_or(1_000)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use super::{probe, start, Frames, BUFFER_BYTES};
    use crate::listen::ListenError;

    type Levels = (u16, u16, u16, u64);

    fn samples(values: &[i16]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    #[test]
    fn probe_reports_levels_or_a_typed_failure() {
        let loud = samples(&[16_384, 0, i16::MAX, -16_384]);
        let cases: [(u64, Vec<u8>, Result<Levels, ListenError>); 3] = [
            (
                0,
                loud.clone(),
                Err(ListenError::InputUnavailable(
                    "probe duration must be greater than zero".to_owned(),
                )),
            ),
            (
                500,
                Vec::new(),
                Err(ListenError::CaptureFailed(
                    "audio source returned no samples".to_owned(),
                )),
            ),
            (500, loud, Ok((999, 612, 750, 1))),
        ];
        for (duration_ms, pcm, expected) in cases {
            let result = probe("mic".to_owned(), duration_ms, |_| {
                Ok((Cursor::new(pcm.clone()), || {}))
            })
            .map(|report| {
                (
                    report.peak_permille,
                    report.rms_permille,
                    report.nonzero_permille,
                    report.clipped_samples,
                )
            });
            assert_eq!(result, expected, "duration {duration_ms}");
        }
    }

    #[test]
    fn a_stream_that_ends_unasked_reports_a_capture_failure() {
        let (frames, received) = mpsc::sync_channel(4);
        let (events, failures) = crossbeam_channel::unbounded();
        let dropped = Arc::new(AtomicU64::new(0));
        let input = start(
            Cursor::new(vec![0_u8; BUFFER_BYTES * 2]),
            || {},
            "mic",
            Frames {
                session_started_at: Instant::now(),
                frames,
                dropped: dropped.clone(),
                events,
            },
        )
        .unwrap();

        let failure = failures.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(failure, Err(ListenError::CaptureFailed(_))));
        assert_eq!(received.try_iter().count(), 2);
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
        drop(input);
    }
}
