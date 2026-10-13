use std::process::Command;

use qol_audio::devices::{self, Device, Direction};
use qol_config::contract::{audio_device_picture, AudioDirection};

use super::super::stream;
use super::{command_error, default_source_name, spawn_capture, terminate_child};
use crate::listen::{AudioInputDevice, AudioInputProbe, AudioInputRequest, ListenError};

fn is_monitor(device: &Device) -> bool {
    device.monitor_of_sink.is_some()
        || device
            .properties
            .get("device.class")
            .is_some_and(|class| class == "monitor")
        || device.name.ends_with(".monitor")
}

fn device_picture(device: &Device) -> Option<String> {
    let entry = serde_json::json!({
        "name": &device.name,
        "properties": &device.properties,
    });
    audio_device_picture(&entry, AudioDirection::Input).map(str::to_owned)
}

pub(crate) fn audio_input_devices() -> Result<Vec<AudioInputDevice>, ListenError> {
    let sources = devices::list(Direction::Input).map_err(|error| {
        ListenError::InputUnavailable(format!("could not list audio inputs: {error}"))
    })?;
    let default = default_source_name()?;
    Ok(map_input_devices(sources, &default))
}

pub(crate) fn verify_audio_input() -> Result<(), ListenError> {
    let output = Command::new("parec")
        .arg("--version")
        .output()
        .map_err(|error| {
            ListenError::InputUnavailable(format!(
                "could not run parec: {error}; install PulseAudio utilities"
            ))
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(ListenError::InputUnavailable(command_error(
        "parec is installed but unavailable",
        &output.stderr,
    )))
}

pub(crate) fn probe_audio_input(
    input: AudioInputRequest,
    duration_ms: u64,
) -> Result<AudioInputProbe, ListenError> {
    let device_id = input.device_id.map_or_else(default_source_name, Ok)?;
    stream::probe(device_id, duration_ms, |device_name| {
        let (stdout, mut child) = spawn_capture(device_name)?;
        Ok((stdout, move || terminate_child(&mut child)))
    })
}

fn map_input_devices(devices: Vec<Device>, default: &str) -> Vec<AudioInputDevice> {
    devices
        .into_iter()
        .filter(|device| !is_monitor(device))
        .map(|device| AudioInputDevice {
            picture: device_picture(&device),
            is_default: device.name == default,
            id: device.name,
            label: device.description,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::map_input_devices;
    use qol_audio::devices::{Device, State};

    fn device(
        name: &str,
        description: &str,
        properties: &[(&str, &str)],
        monitor_of_sink: Option<u32>,
    ) -> Device {
        Device {
            index: 1,
            name: name.to_owned(),
            description: description.to_owned(),
            properties: properties
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
            active_port: None,
            monitor_of_sink,
            state: State::Unknown,
        }
    }

    #[test]
    fn source_inventory_excludes_output_monitors_and_marks_default() {
        let devices = vec![
            device(
                "mic.one",
                "Desk microphone",
                &[("device.class", "sound"), ("device.form_factor", "webcam")],
                None,
            ),
            device(
                "speaker.monitor",
                "Speaker monitor",
                &[("device.class", "monitor")],
                None,
            ),
            device("sink.five.monitor-output", "Sink monitor", &[], Some(5)),
        ];

        let mapped = map_input_devices(devices, "mic.one");

        assert_eq!(mapped.len(), 1);
        assert_eq!(mapped[0].id, "mic.one");
        assert!(mapped[0].is_default);
        assert_eq!(mapped[0].picture.as_deref(), Some("webcam"));
    }
}
