use std::sync::atomic::AtomicU64;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::Sender;
use qol_audio::capture::{Capture, Pcm16Format};
use qol_audio::default_output;
use qol_audio::devices::{self, AudioDevice, Direction};
use qol_audio::AudioError;

use crate::audio::AudioFrame;

use super::super::{
    ActiveAudioInput, AudioInput, AudioInputDevice, AudioInputInfo, AudioInputProbe,
    AudioInputRequest, ListenError, ListenMessage,
};
use super::stream::{self, Frames, CHANNELS, FORMAT, SAMPLE_RATE};

const PCM: Pcm16Format = Pcm16Format {
    sample_rate: SAMPLE_RATE,
    channels: CHANNELS,
};

pub(crate) struct PlatformAudioInput {
    requested_device: Option<String>,
}

impl PlatformAudioInput {
    pub(crate) fn new(request: AudioInputRequest) -> Self {
        Self {
            requested_device: request.device_id,
        }
    }
}

impl AudioInput for PlatformAudioInput {
    fn info(&self) -> Result<AudioInputInfo, ListenError> {
        Ok(AudioInputInfo {
            device_name: self
                .requested_device
                .clone()
                .map_or_else(stream::default_input, Ok)?,
            format: FORMAT,
        })
    }

    fn start(
        &self,
        session_started_at: Instant,
        frames: SyncSender<AudioFrame>,
        dropped: Arc<AtomicU64>,
        events: Sender<ListenMessage>,
    ) -> Result<Box<dyn ActiveAudioInput>, ListenError> {
        let capture = open_capture(self.requested_device.as_deref())?;
        let device_name = capture.device().to_owned();
        let stopper = capture.stopper();
        stream::start(
            capture,
            move || stopper.stop(),
            &device_name,
            Frames {
                session_started_at,
                frames,
                dropped,
                events,
            },
        )
    }
}

pub(crate) fn audio_input_devices() -> Result<Vec<AudioInputDevice>, ListenError> {
    let inputs = devices::list_inputs().map_err(|error| {
        ListenError::InputUnavailable(format!("could not list audio inputs: {error}"))
    })?;
    let default = default_output::effective(Direction::Input).map_err(|error| {
        ListenError::InputUnavailable(format!(
            "could not resolve the default audio source: {error}"
        ))
    })?;
    Ok(map_input_devices(inputs, default.as_deref()))
}

pub(crate) fn verify_audio_input() -> Result<(), ListenError> {
    devices::list_inputs().map(drop).map_err(|error| {
        ListenError::InputUnavailable(format!("the Windows audio service did not answer: {error}"))
    })
}

pub(crate) fn probe_audio_input(
    input: AudioInputRequest,
    duration_ms: u64,
) -> Result<AudioInputProbe, ListenError> {
    let device_id = input.device_id.map_or_else(stream::default_input, Ok)?;
    stream::probe(device_id, duration_ms, |device_name| {
        let capture = open_capture(Some(device_name))?;
        let stopper = capture.stopper();
        Ok((capture, move || stopper.stop()))
    })
}

fn open_capture(device: Option<&str>) -> Result<Capture, ListenError> {
    Capture::open(device, PCM).map_err(|error| {
        if error == AudioError::no_default(Direction::Input) {
            return ListenError::NoInputDevice;
        }
        ListenError::InputUnavailable(error.to_string())
    })
}

fn map_input_devices(inputs: Vec<AudioDevice>, default: Option<&str>) -> Vec<AudioInputDevice> {
    inputs
        .into_iter()
        .map(|device| AudioInputDevice {
            is_default: default == Some(device.identity.as_str()),
            id: device.identity.as_str().to_owned(),
            label: device.label,
            picture: Some(device.picture),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use qol_audio::devices::{AudioDevice, Identity, Kind};

    use super::map_input_devices;

    fn input(id: &str, label: &str, picture: &str) -> AudioDevice {
        AudioDevice {
            identity: Identity::from_raw(id),
            value: id.to_owned(),
            label: label.to_owned(),
            picture: picture.to_owned(),
            kind: Kind::Unknown,
        }
    }

    #[test]
    fn endpoints_map_to_inputs_and_mark_the_default() {
        let cases = [
            (Some("{mic.usb}"), [false, true]),
            (Some("{gone}"), [false, false]),
            (None, [false, false]),
        ];
        for (default, expected) in cases {
            let mapped = map_input_devices(
                vec![
                    input("{mic.array}", "Microphone Array", "mic-default"),
                    input("{mic.usb}", "USB Microphone", "usb-mic"),
                ],
                default,
            );
            let defaults = mapped
                .iter()
                .map(|device| device.is_default)
                .collect::<Vec<_>>();
            assert_eq!(defaults, expected, "default {default:?}");
            assert_eq!(mapped[1].id, "{mic.usb}");
            assert_eq!(mapped[1].label, "USB Microphone");
            assert_eq!(mapped[1].picture.as_deref(), Some("usb-mic"));
        }
    }
}
