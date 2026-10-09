use std::io::Cursor;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use pulseaudio::protocol::stream::{BufferAttr, StreamFlags};
use pulseaudio::protocol::{self, ChannelMap, RecordStreamParams, SampleFormat, SampleSpec};

use crate::devices::Direction;
use crate::AudioError;

use super::connection::{Closer, Connection};
use super::default_output::default_node;

const METER_RATE: u32 = 25;
const FRAGMENT_BYTES: u32 = 4;
const SAMPLE_BYTES: usize = 4;
const CONTROL_CHANNEL: u32 = u32::MAX;

pub(crate) struct Meter {
    device: String,
    includes_volume: bool,
    peak: Arc<AtomicU32>,
    closer: Closer,
    worker: Option<JoinHandle<()>>,
}

impl Meter {
    pub(crate) fn open(direction: Direction) -> Result<Self, AudioError> {
        let mut connection = Connection::connect()?;
        let target = target(&mut connection, direction)?;
        let reply = connection.request::<protocol::CreateRecordStreamReply>(
            &protocol::Command::CreateRecordStream(record_params(target.source_index)),
        )?;
        let closer = connection.closer()?;
        let peak = Arc::new(AtomicU32::new(0f32.to_bits()));
        let shared = Arc::clone(&peak);
        let worker = std::thread::Builder::new()
            .name("qol-audio-meter".to_owned())
            .spawn(move || run(connection, reply.channel, &shared))
            .map_err(|error| {
                AudioError::Operation(format!("cannot start the level meter: {error}"))
            })?;
        Ok(Self {
            device: target.device,
            includes_volume: target.includes_volume,
            peak,
            closer,
            worker: Some(worker),
        })
    }

    pub(crate) fn peak(&self) -> f32 {
        f32::from_bits(self.peak.load(Ordering::Relaxed))
    }

    pub(crate) fn device(&self) -> &str {
        &self.device
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }

    pub(crate) fn peak_before_volume(&self, percent: u32) -> f32 {
        before_volume(self.peak(), percent, self.includes_volume)
    }
}

fn before_volume(peak: f32, percent: u32, includes_volume: bool) -> f32 {
    if !includes_volume {
        return peak;
    }
    let gain = (percent as f32 / 100.0).powi(3);
    if gain <= 0.0 {
        return 0.0;
    }
    (peak / gain).min(1.0)
}

impl Drop for Meter {
    fn drop(&mut self) {
        self.closer.close();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Target {
    device: String,
    source_index: u32,
    includes_volume: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    Ended,
    Silenced,
    Other,
}

fn target(connection: &mut Connection, direction: Direction) -> Result<Target, AudioError> {
    let node =
        default_node(connection, direction)?.ok_or_else(|| super::mute::no_default(direction))?;
    let source_index = match direction {
        Direction::Output => node.monitor_source_index.ok_or_else(|| {
            AudioError::Operation(format!(
                "the output '{}' has no monitor to measure",
                node.name
            ))
        })?,
        Direction::Input => node.index,
    };
    let includes_volume = !node.hardware_volume;
    Ok(Target {
        device: node.name,
        source_index,
        includes_volume,
    })
}

fn record_params(source_index: u32) -> RecordStreamParams {
    let mut props = protocol::Props::new();
    props.set(protocol::Prop::MediaName, c"qol-audio level meter");
    RecordStreamParams {
        sample_spec: SampleSpec {
            format: SampleFormat::Float32Le,
            channels: 1,
            sample_rate: METER_RATE,
        },
        channel_map: ChannelMap::mono(),
        source_index: Some(source_index),
        buffer_attr: BufferAttr {
            max_length: u32::MAX,
            fragment_size: FRAGMENT_BYTES,
            ..Default::default()
        },
        flags: StreamFlags {
            no_move: true,
            peak_detect: true,
            adjust_latency: true,
            no_inhibit_auto_suspend: true,
            ..Default::default()
        },
        props,
        ..Default::default()
    }
}

fn run(mut connection: Connection, channel: u32, peak: &AtomicU32) {
    while let Ok((descriptor, payload)) = connection.read_message() {
        if descriptor.channel == channel {
            store_latest(peak, &payload);
            continue;
        }
        if descriptor.channel != CONTROL_CHANNEL {
            continue;
        }
        match control_event(&payload, channel, connection.version()) {
            Event::Ended => break,
            Event::Silenced => store(peak, 0.0),
            Event::Other => {}
        }
    }
    store(peak, 0.0);
}

fn store_latest(peak: &AtomicU32, payload: &[u8]) {
    if let Some(level) = latest_peak(payload) {
        store(peak, level);
    }
}

fn store(peak: &AtomicU32, level: f32) {
    peak.store(level.to_bits(), Ordering::Relaxed);
}

fn latest_peak(payload: &[u8]) -> Option<f32> {
    let (samples, _) = payload.as_chunks::<SAMPLE_BYTES>();
    let value = f32::from_le_bytes(*samples.last()?);
    if value.is_nan() {
        return Some(0.0);
    }
    Some(value.abs().min(1.0))
}

fn control_event(payload: &[u8], channel: u32, version: u16) -> Event {
    let Ok((_, command)) = protocol::Command::read_tag_prefixed(&mut Cursor::new(payload), version)
    else {
        return Event::Other;
    };
    match command {
        protocol::Command::RecordStreamKilled(killed) if killed == channel => Event::Ended,
        protocol::Command::RecordStreamSuspended(params)
            if params.stream_index == channel && params.suspended =>
        {
            Event::Silenced
        }
        _ => Event::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples(values: &[f32]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect()
    }

    fn encoded(command: &protocol::Command) -> Vec<u8> {
        let mut buffer = Vec::new();
        command
            .write_tag_prefixed(0, &mut buffer, protocol::MAX_VERSION)
            .expect("encode a control command");
        buffer
    }

    #[test]
    fn a_peak_measured_after_a_software_volume_is_reported_before_it() {
        assert!((before_volume(0.108, 60, true) - 0.5).abs() < 0.001);
        assert_eq!(before_volume(0.5, 100, true), 0.5);
        assert_eq!(before_volume(0.9, 30, true), 1.0);
        assert_eq!(before_volume(0.4, 0, true), 0.0);
        assert_eq!(before_volume(0.4, 30, false), 0.4);
    }

    #[test]
    fn the_latest_whole_sample_is_the_peak() {
        assert_eq!(latest_peak(&samples(&[0.1, 0.25, 0.5])), Some(0.5));

        let mut torn = samples(&[0.3]);
        torn.extend_from_slice(&[0, 0]);
        assert_eq!(latest_peak(&torn), Some(0.3));
    }

    #[test]
    fn peaks_are_magnitudes_clamped_to_full_scale() {
        assert_eq!(latest_peak(&samples(&[-0.75])), Some(0.75));
        assert_eq!(latest_peak(&samples(&[1.6])), Some(1.0));
        assert_eq!(latest_peak(&samples(&[f32::NAN])), Some(0.0));
    }

    #[test]
    fn a_payload_shorter_than_one_sample_keeps_the_last_peak() {
        assert_eq!(latest_peak(&[]), None);
        assert_eq!(latest_peak(&[0, 0, 0]), None);

        let peak = AtomicU32::new(0.4f32.to_bits());
        store_latest(&peak, &[0, 0]);
        assert_eq!(f32::from_bits(peak.load(Ordering::Relaxed)), 0.4);
    }

    #[test]
    fn only_this_streams_kill_ends_the_meter_and_suspension_silences_it() {
        let channel = 3;
        assert_eq!(
            control_event(
                &encoded(&protocol::Command::RecordStreamKilled(channel)),
                channel,
                protocol::MAX_VERSION
            ),
            Event::Ended
        );
        assert_eq!(
            control_event(
                &encoded(&protocol::Command::RecordStreamKilled(channel + 1)),
                channel,
                protocol::MAX_VERSION
            ),
            Event::Other
        );
        assert_eq!(
            control_event(
                &encoded(&protocol::Command::RecordStreamSuspended(
                    protocol::StreamSuspendedParams {
                        stream_index: channel,
                        suspended: true,
                    }
                )),
                channel,
                protocol::MAX_VERSION
            ),
            Event::Silenced
        );
        assert_eq!(
            control_event(&[0xff], channel, protocol::MAX_VERSION),
            Event::Other
        );
    }
}
