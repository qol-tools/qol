use std::collections::BTreeMap;
use std::io::Read;
use std::process::Command;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use serde::Deserialize;

use super::{
    capture_command, command_error, default_source_name, terminate_child, CAPTURE_START_TIMEOUT,
    SAMPLE_RATE,
};
use crate::listen::{AudioInputDevice, AudioInputProbe, AudioInputRequest, ListenError};

#[derive(Deserialize)]
struct PulseSource {
    name: String,
    description: String,
    #[serde(default)]
    monitor_of_sink: Option<serde_json::Value>,
    #[serde(default)]
    properties: BTreeMap<String, String>,
}

impl PulseSource {
    fn is_monitor(&self) -> bool {
        self.monitor_of_sink.is_some()
            || self
                .properties
                .get("device.class")
                .is_some_and(|class| class == "monitor")
            || self.name.ends_with(".monitor")
    }
}

pub(crate) fn audio_input_devices() -> Result<Vec<AudioInputDevice>, ListenError> {
    let output = Command::new("pactl")
        .args(["--format=json", "list", "sources"])
        .output()
        .map_err(|error| {
            ListenError::InputUnavailable(format!(
                "could not run pactl: {error}; install PulseAudio utilities"
            ))
        })?;
    if !output.status.success() {
        return Err(ListenError::InputUnavailable(command_error(
            "pactl could not list audio sources",
            &output.stderr,
        )));
    }
    let default = default_source_name()?;
    parse_input_devices(&output.stdout, &default)
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
    if duration_ms == 0 {
        return Err(ListenError::InputUnavailable(
            "probe duration must be greater than zero".to_owned(),
        ));
    }
    let device_id = input.device_id.map_or_else(default_source_name, Ok)?;
    let pcm = capture_probe_pcm(&device_id, duration_ms)?;
    Ok(probe_report(device_id, pcm))
}

fn parse_input_devices(json: &[u8], default: &str) -> Result<Vec<AudioInputDevice>, ListenError> {
    let sources = serde_json::from_slice::<Vec<PulseSource>>(json)
        .map_err(|error| ListenError::InputUnavailable(format!("invalid pactl output: {error}")))?;
    Ok(sources
        .into_iter()
        .filter(|source| !source.is_monitor())
        .map(|source| AudioInputDevice {
            is_default: source.name == default,
            id: source.name,
            label: source.description,
        })
        .collect())
}

fn capture_probe_pcm(device_name: &str, duration_ms: u64) -> Result<Vec<u8>, ListenError> {
    let target_bytes = u64::from(SAMPLE_RATE)
        .saturating_mul(2)
        .saturating_mul(duration_ms)
        / 1_000;
    let mut child = capture_command(device_name).spawn().map_err(|error| {
        ListenError::InputUnavailable(format!(
            "could not start parec: {error}; install PulseAudio utilities"
        ))
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        ListenError::InputUnavailable("parec did not provide an audio stream".to_owned())
    })?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut pcm = Vec::new();
        let result = stdout.take(target_bytes).read_to_end(&mut pcm).map(|_| pcm);
        let _ = sender.send(result);
    });
    let timeout = Duration::from_millis(duration_ms).saturating_add(CAPTURE_START_TIMEOUT);
    let received = receiver.recv_timeout(timeout);
    terminate_child(&mut child);
    let _ = reader.join();
    match received {
        Ok(Ok(pcm)) if !pcm.is_empty() => Ok(pcm),
        Ok(Ok(_)) => Err(ListenError::CaptureFailed(
            "audio source returned no samples".to_owned(),
        )),
        Ok(Err(error)) => Err(ListenError::CaptureFailed(error.to_string())),
        Err(RecvTimeoutError::Disconnected) => Err(ListenError::CaptureFailed(
            "audio probe stopped before producing samples".to_owned(),
        )),
        Err(RecvTimeoutError::Timeout) => Err(ListenError::InputUnavailable(format!(
            "source '{device_name}' produced no audio during the probe"
        ))),
    }
}

fn probe_report(device_id: String, pcm: Vec<u8>) -> AudioInputProbe {
    let samples = pcm
        .chunks_exact(2)
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
    use super::parse_input_devices;

    #[test]
    fn source_inventory_excludes_output_monitors_and_marks_default() {
        let json = br#"[
          {
            "name":"mic.one",
            "description":"Desk microphone",
            "monitor_of_sink":null,
            "properties":{"device.class":"sound"}
          },
          {
            "name":"speaker.monitor",
            "description":"Speaker monitor",
            "monitor_of_sink":null,
            "properties":{"device.class":"monitor"}
          }
        ]"#;

        let devices = parse_input_devices(json, "mic.one").unwrap();

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "mic.one");
        assert!(devices[0].is_default);
    }
}
