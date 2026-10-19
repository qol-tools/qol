use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::devices::Direction;
use crate::AudioError;

use super::default_output::default_device;
use super::hal::{self, ObjectId, Property};
use super::identity;
use super::io::{self, Block, BufferList, Listening};
use super::tap::SystemTap;

const WINDOWS_PER_SECOND: f64 = 25.0;
const FALLBACK_SAMPLE_RATE: f64 = 48_000.0;

pub(crate) struct Meter {
    device: String,
    listening: Listening,
    _system_tap: Option<SystemTap>,
    borrowed_microphone: Option<ObjectId>,
    peaks: Box<Peaks>,
}

impl Meter {
    pub(crate) fn open(direction: Direction) -> Result<Self, AudioError> {
        let id = default_device(direction)?.ok_or_else(|| AudioError::no_default(direction))?;
        let device = hal::read_text(id, Property::global(hal::DEVICE_UID))?;
        match direction {
            Direction::Input => {
                let bluetooth = identity::is_bluetooth(id);
                if bluetooth && !another_app_records(id) {
                    return Err(AudioError::Operation(
                        "measuring a Bluetooth microphone no app is recording from would switch the headset to its call profile"
                            .to_owned(),
                    ));
                }
                Self::listen(device, id, None, bluetooth.then_some(id))
            }
            Direction::Output => {
                let system_tap = SystemTap::create()?;
                Self::listen(device, system_tap.device(), Some(system_tap), None)
            }
        }
    }

    fn listen(
        device: String,
        source: ObjectId,
        system_tap: Option<SystemTap>,
        borrowed_microphone: Option<ObjectId>,
    ) -> Result<Self, AudioError> {
        io::require_float_samples(source, Direction::Input)?;
        let peaks = Box::new(Peaks::new(window_size(sample_rate(source))));
        let client = (&raw const *peaks).cast_mut().cast::<c_void>();
        let listening = Listening::start(source, listen, client)?;
        Ok(Self {
            device,
            listening,
            _system_tap: system_tap,
            borrowed_microphone,
            peaks,
        })
    }

    pub(crate) fn peak(&self) -> f32 {
        f32::from_bits(self.peaks.published.load(Ordering::Relaxed))
    }

    pub(crate) fn device(&self) -> &str {
        &self.device
    }

    pub(crate) fn is_finished(&self) -> bool {
        let alive = hal::read_u32(self.listening.device(), Property::global(hal::IS_ALIVE))
            .is_ok_and(|alive| alive != 0);
        !alive
            || self
                .borrowed_microphone
                .is_some_and(|microphone| !another_app_records(microphone))
    }

    pub(crate) fn peak_before_volume(&self, percent: u32) -> f32 {
        let _ = percent;
        self.peak()
    }
}

fn another_app_records(device: ObjectId) -> bool {
    hal::read_ids(hal::SYSTEM, Property::global(hal::PROCESSES)).is_ok_and(|processes| {
        let recordings: Vec<Recording> = processes.into_iter().filter_map(recording).collect();
        records_elsewhere(&recordings, device, std::process::id())
    })
}

#[derive(Debug, Clone, PartialEq)]
struct Recording {
    pid: u32,
    devices: Vec<ObjectId>,
}

fn recording(process: ObjectId) -> Option<Recording> {
    let running = hal::read_u32(process, Property::global(hal::PROCESS_IS_RUNNING_INPUT)).ok()?;
    if running == 0 {
        return None;
    }
    Some(Recording {
        pid: hal::read_u32(process, Property::global(hal::PROCESS_PID)).ok()?,
        devices: hal::read_ids(
            process,
            Property::scoped(hal::PROCESS_DEVICES, Direction::Input),
        )
        .ok()?,
    })
}

fn records_elsewhere(recordings: &[Recording], device: ObjectId, own_pid: u32) -> bool {
    recordings
        .iter()
        .any(|recording| recording.pid != own_pid && recording.devices.contains(&device))
}

fn sample_rate(id: ObjectId) -> f64 {
    hal::read_into(id, Property::global(hal::NOMINAL_SAMPLE_RATE), 0f64)
        .ok()
        .filter(|rate| *rate > 0.0)
        .unwrap_or(FALLBACK_SAMPLE_RATE)
}

fn window_size(sample_rate: f64) -> u32 {
    ((sample_rate / WINDOWS_PER_SECOND).round() as u32).max(1)
}

struct Peaks {
    published: AtomicU32,
    window_peak: AtomicU32,
    window_frames: AtomicU32,
    size: u32,
}

impl Peaks {
    fn new(size: u32) -> Self {
        Self {
            published: AtomicU32::new(0f32.to_bits()),
            window_peak: AtomicU32::new(0f32.to_bits()),
            window_frames: AtomicU32::new(0),
            size,
        }
    }

    fn record(&self, block: Block) {
        let window = Window {
            peak: f32::from_bits(self.window_peak.load(Ordering::Relaxed)),
            frames: self.window_frames.load(Ordering::Relaxed),
        };
        let (next, published) = advance(window, block, self.size);
        self.window_peak
            .store(next.peak.to_bits(), Ordering::Relaxed);
        self.window_frames.store(next.frames, Ordering::Relaxed);
        if let Some(peak) = published {
            self.published.store(peak.to_bits(), Ordering::Relaxed);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Window {
    peak: f32,
    frames: u32,
}

const EMPTY: Window = Window {
    peak: 0.0,
    frames: 0,
};

fn advance(window: Window, block: Block, size: u32) -> (Window, Option<f32>) {
    let peak = window.peak.max(block.loudest);
    let frames = window.frames.saturating_add(block.frames);
    if frames >= size {
        return (EMPTY, Some(peak));
    }
    (Window { peak, frames }, None)
}

unsafe extern "C" fn listen(
    _device: ObjectId,
    _now: *const c_void,
    input: *const BufferList,
    _input_time: *const c_void,
    _output: *mut BufferList,
    _output_time: *const c_void,
    client: *mut c_void,
) -> i32 {
    if let Some(peaks) = unsafe { client.cast::<Peaks>().as_ref() } {
        peaks.record(unsafe { io::block(input) });
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(loudest: f32, frames: u32) -> Block {
        Block { loudest, frames }
    }

    fn recording(pid: u32, devices: &[ObjectId]) -> Recording {
        Recording {
            pid,
            devices: devices.to_vec(),
        }
    }

    #[test]
    fn only_another_process_recording_from_the_device_counts() {
        let own = 400;
        let headset = 88;
        let built_in = 77;
        let cases: [(&str, Vec<Recording>, bool); 5] = [
            ("nobody records", vec![], false),
            (
                "only this process records",
                vec![recording(own, &[headset])],
                false,
            ),
            (
                "another app records elsewhere",
                vec![recording(51, &[built_in])],
                false,
            ),
            (
                "another app records the headset",
                vec![recording(51, &[headset])],
                true,
            ),
            (
                "this process and a call app record",
                vec![
                    recording(own, &[headset]),
                    recording(51, &[built_in, headset]),
                ],
                true,
            ),
        ];
        for (name, recordings, expected) in cases {
            assert_eq!(
                records_elsewhere(&recordings, headset, own),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn a_window_is_forty_milliseconds_of_frames() {
        assert_eq!(window_size(48_000.0), 1_920);
        assert_eq!(window_size(44_100.0), 1_764);
        assert_eq!(window_size(16_000.0), 640);
        assert_eq!(window_size(0.0), 1);
    }

    #[test]
    fn the_loudest_block_is_published_once_the_window_fills() {
        let (window, published) = advance(EMPTY, block(0.2, 512), 1_000);
        assert_eq!(published, None);
        let (window, published) = advance(window, block(0.7, 256), 1_000);
        assert_eq!(published, None);
        assert_eq!(
            window,
            Window {
                peak: 0.7,
                frames: 768
            }
        );
        let (window, published) = advance(window, block(0.1, 512), 1_000);
        assert_eq!(published, Some(0.7));
        assert_eq!(window, EMPTY);
    }

    #[test]
    fn the_peaks_publish_through_their_atomics() {
        let peaks = Peaks::new(4);
        peaks.record(block(0.3, 2));
        assert_eq!(f32::from_bits(peaks.published.load(Ordering::Relaxed)), 0.0);
        peaks.record(block(0.5, 2));
        assert_eq!(f32::from_bits(peaks.published.load(Ordering::Relaxed)), 0.5);
        peaks.record(block(0.1, 4));
        assert_eq!(f32::from_bits(peaks.published.load(Ordering::Relaxed)), 0.1);
    }
}
