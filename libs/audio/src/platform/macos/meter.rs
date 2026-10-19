use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::devices::{Direction, State};
use crate::AudioError;

use super::default_output::default_device;
use super::hal::{self, ObjectId, Property};
use super::identity;
use super::io::{self, Block, BufferList, Listening};

const WINDOWS_PER_SECOND: f64 = 25.0;
const FALLBACK_SAMPLE_RATE: f64 = 48_000.0;

pub(crate) struct Meter {
    device: String,
    listening: Listening,
    tap: Box<Tap>,
}

impl Meter {
    pub(crate) fn open(direction: Direction) -> Result<Self, AudioError> {
        if direction == Direction::Output {
            return Err(AudioError::Unsupported);
        }
        let id = default_device(direction)?.ok_or_else(|| AudioError::no_default(direction))?;
        if identity::is_bluetooth(id) && identity::state_of(id) != State::Running {
            return Err(AudioError::Operation(
                "measuring an idle Bluetooth microphone would switch the headset to its call profile"
                    .to_owned(),
            ));
        }
        let device = hal::read_text(id, Property::global(hal::DEVICE_UID))?;
        io::require_float_samples(id, direction)?;
        let tap = Box::new(Tap::new(window_size(sample_rate(id))));
        let client = (&raw const *tap).cast_mut().cast::<c_void>();
        let listening = Listening::start(id, listen, client)?;
        Ok(Self {
            device,
            listening,
            tap,
        })
    }

    pub(crate) fn peak(&self) -> f32 {
        f32::from_bits(self.tap.published.load(Ordering::Relaxed))
    }

    pub(crate) fn device(&self) -> &str {
        &self.device
    }

    pub(crate) fn is_finished(&self) -> bool {
        !hal::read_u32(self.listening.device(), Property::global(hal::IS_ALIVE))
            .is_ok_and(|alive| alive != 0)
    }

    pub(crate) fn peak_before_volume(&self, percent: u32) -> f32 {
        let _ = percent;
        self.peak()
    }
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

struct Tap {
    published: AtomicU32,
    window_peak: AtomicU32,
    window_frames: AtomicU32,
    size: u32,
}

impl Tap {
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
    if let Some(tap) = unsafe { client.cast::<Tap>().as_ref() } {
        tap.record(unsafe { io::block(input) });
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(loudest: f32, frames: u32) -> Block {
        Block { loudest, frames }
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
    fn the_tap_publishes_through_its_atomics() {
        let tap = Tap::new(4);
        tap.record(block(0.3, 2));
        assert_eq!(f32::from_bits(tap.published.load(Ordering::Relaxed)), 0.0);
        tap.record(block(0.5, 2));
        assert_eq!(f32::from_bits(tap.published.load(Ordering::Relaxed)), 0.5);
        tap.record(block(0.1, 4));
        assert_eq!(f32::from_bits(tap.published.load(Ordering::Relaxed)), 0.1);
    }
}
