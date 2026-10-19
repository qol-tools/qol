use std::ffi::c_void;

use crate::devices::Direction;
use crate::AudioError;

use super::hal::{self, ObjectId, Property};

const LINEAR_PCM: [u8; 4] = *b"lpcm";
const FLOAT_FLAG: u32 = 1;
const FLOAT_BITS: u32 = 32;
const SAMPLE_BYTES: usize = std::mem::size_of::<f32>();

pub(super) type IoProc = unsafe extern "C" fn(
    device: ObjectId,
    now: *const c_void,
    input: *const BufferList,
    input_time: *const c_void,
    output: *mut BufferList,
    output_time: *const c_void,
    client: *mut c_void,
) -> i32;

#[repr(C)]
struct Buffer {
    channels: u32,
    bytes: u32,
    data: *mut c_void,
}

#[repr(C)]
pub(super) struct BufferList {
    count: u32,
    buffers: [Buffer; 1],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct StreamFormat {
    sample_rate: f64,
    format: u32,
    flags: u32,
    bytes_per_packet: u32,
    frames_per_packet: u32,
    bytes_per_frame: u32,
    channels_per_frame: u32,
    bits_per_channel: u32,
    reserved: u32,
}

#[link(name = "CoreAudio", kind = "framework")]
extern "C" {
    fn AudioDeviceCreateIOProcID(
        device: ObjectId,
        proc: IoProc,
        client: *mut c_void,
        id: *mut *mut c_void,
    ) -> i32;
    fn AudioDeviceDestroyIOProcID(device: ObjectId, id: *mut c_void) -> i32;
    fn AudioDeviceStart(device: ObjectId, id: *mut c_void) -> i32;
    fn AudioDeviceStop(device: ObjectId, id: *mut c_void) -> i32;
}

pub(super) struct Listening {
    device: ObjectId,
    id: *mut c_void,
}

unsafe impl Send for Listening {}

impl Listening {
    pub(super) fn start(
        device: ObjectId,
        proc: IoProc,
        client: *mut c_void,
    ) -> Result<Self, AudioError> {
        let mut id = std::ptr::null_mut();
        let status = unsafe { AudioDeviceCreateIOProcID(device, proc, client, &mut id) };
        if status != 0 {
            return Err(refused("cannot listen to the device", status));
        }
        let listening = Self { device, id };
        let status = unsafe { AudioDeviceStart(device, id) };
        if status != 0 {
            return Err(refused("cannot start listening to the device", status));
        }
        Ok(listening)
    }

    pub(super) fn device(&self) -> ObjectId {
        self.device
    }
}

impl Drop for Listening {
    fn drop(&mut self) {
        unsafe {
            AudioDeviceStop(self.device, self.id);
            AudioDeviceDestroyIOProcID(self.device, self.id);
        }
    }
}

fn refused(what: &str, status: i32) -> AudioError {
    AudioError::Operation(format!(
        "{what}: Core Audio answered {}",
        hal::status_text(status)
    ))
}

pub(super) fn require_float_samples(
    device: ObjectId,
    direction: Direction,
) -> Result<(), AudioError> {
    let streams = hal::read_ids(device, Property::scoped(hal::STREAMS, direction))?;
    if streams.is_empty() {
        return Err(AudioError::Operation(
            "the device has no streams to listen to".to_owned(),
        ));
    }
    for stream in streams {
        let format = hal::read_into(
            stream,
            Property::global(hal::VIRTUAL_FORMAT),
            StreamFormat::default(),
        )?;
        if !carries_float_samples(&format) {
            return Err(AudioError::Operation(format!(
                "the device delivers '{}' samples of {} bits, not 32-bit float",
                String::from_utf8_lossy(&format.format.to_be_bytes()),
                format.bits_per_channel
            )));
        }
    }
    Ok(())
}

fn carries_float_samples(format: &StreamFormat) -> bool {
    format.format == u32::from_be_bytes(LINEAR_PCM)
        && format.flags & FLOAT_FLAG != 0
        && format.bits_per_channel == FLOAT_BITS
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Block {
    pub(super) loudest: f32,
    pub(super) frames: u32,
}

pub(super) unsafe fn block(list: *const BufferList) -> Block {
    let mut block = Block {
        loudest: 0.0,
        frames: 0,
    };
    let Some(list) = (unsafe { list.as_ref() }) else {
        return block;
    };
    let buffers = unsafe { std::slice::from_raw_parts(list.buffers.as_ptr(), list.count as usize) };
    for buffer in buffers {
        if buffer.data.is_null() {
            continue;
        }
        let samples = unsafe {
            std::slice::from_raw_parts(
                buffer.data.cast::<f32>(),
                buffer.bytes as usize / SAMPLE_BYTES,
            )
        };
        block.loudest = samples
            .iter()
            .fold(block.loudest, |loudest, sample| loudest.max(sample.abs()));
        let frames = samples.len() / buffer.channels.max(1) as usize;
        block.frames = block.frames.max(u32::try_from(frames).unwrap_or(u32::MAX));
    }
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    struct TwoBuffers {
        count: u32,
        buffers: [Buffer; 2],
    }

    fn buffer(samples: &mut [f32], channels: u32) -> Buffer {
        Buffer {
            channels,
            bytes: u32::try_from(samples.len() * SAMPLE_BYTES).unwrap(),
            data: samples.as_mut_ptr().cast(),
        }
    }

    fn format(format: [u8; 4], flags: u32, bits: u32) -> StreamFormat {
        StreamFormat {
            format: u32::from_be_bytes(format),
            flags,
            bits_per_channel: bits,
            ..StreamFormat::default()
        }
    }

    #[test]
    fn only_32_bit_float_pcm_is_measured() {
        assert!(carries_float_samples(&format(*b"lpcm", 0b1001, 32)));
        assert!(!carries_float_samples(&format(*b"lpcm", 0b1100, 16)));
        assert!(!carries_float_samples(&format(*b"lpcm", 0b1100, 32)));
        assert!(!carries_float_samples(&format(*b"aac ", 0b1001, 32)));
    }

    #[test]
    fn the_shared_structs_match_the_core_audio_layout() {
        assert_eq!(std::mem::size_of::<StreamFormat>(), 40);
        assert_eq!(std::mem::size_of::<BufferList>(), 24);
    }

    #[test]
    fn a_block_is_the_loudest_sample_across_buffers_and_its_frame_count() {
        let missing = unsafe { block(std::ptr::null()) };
        assert_eq!(
            missing,
            Block {
                loudest: 0.0,
                frames: 0
            }
        );

        let mut interleaved = [0.2f32, -0.9, 0.1, 0.4];
        let list = BufferList {
            count: 1,
            buffers: [buffer(&mut interleaved, 2)],
        };
        let measured = unsafe { block(&list) };
        assert_eq!(
            measured,
            Block {
                loudest: 0.9,
                frames: 2
            }
        );

        let mut left = [0.1f32, -0.6, 0.2];
        let mut right = [0.3f32, 0.25, -0.05];
        let planar = TwoBuffers {
            count: 2,
            buffers: [buffer(&mut left, 1), buffer(&mut right, 1)],
        };
        let measured = unsafe { block((&raw const planar).cast()) };
        assert_eq!(
            measured,
            Block {
                loudest: 0.6,
                frames: 3
            }
        );
    }
}
