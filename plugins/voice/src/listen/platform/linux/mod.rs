use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::Instant;

use crossbeam_channel::Sender;

use crate::audio::AudioFrame;

use super::super::{
    ActiveAudioInput, AudioInput, AudioInputInfo, AudioInputRequest, ListenError, ListenMessage,
};
use super::stream::{self, Frames, FORMAT};

mod diagnostics;

pub(crate) use diagnostics::{audio_input_devices, probe_audio_input, verify_audio_input};

pub(crate) struct PlatformAudioInput {
    requested_device: Option<String>,
}

impl PlatformAudioInput {
    pub(crate) fn new(request: AudioInputRequest) -> Self {
        Self {
            requested_device: request.device_id,
        }
    }

    fn device_name(&self) -> Result<String, ListenError> {
        self.requested_device
            .clone()
            .map_or_else(default_source_name, Ok)
    }
}

impl AudioInput for PlatformAudioInput {
    fn info(&self) -> Result<AudioInputInfo, ListenError> {
        Ok(AudioInputInfo {
            device_name: self.device_name()?,
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
        let device_name = self.device_name()?;
        let (stdout, mut child) = spawn_capture(&device_name)?;
        stream::start(
            stdout,
            move || terminate_child(&mut child),
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

fn spawn_capture(device_name: &str) -> Result<(ChildStdout, Child), ListenError> {
    let mut child = capture_command(device_name).spawn().map_err(|error| {
        ListenError::InputUnavailable(format!(
            "could not start parec: {error}; install PulseAudio utilities"
        ))
    })?;
    let stdout = child.stdout.take().ok_or_else(|| {
        ListenError::InputUnavailable("parec did not provide an audio stream".to_owned())
    })?;
    Ok((stdout, child))
}

fn default_source_name() -> Result<String, ListenError> {
    if let Some(source) = std::env::var_os("PULSE_SOURCE") {
        let source = source.to_string_lossy().trim().to_owned();
        if !source.is_empty() {
            return Ok(source);
        }
    }
    stream::default_input()
}

fn capture_command(device_name: &str) -> Command {
    let mut command = Command::new("parec");
    command
        .arg(format!("--device={device_name}"))
        .args([
            "--raw",
            "--format=s16le",
            "--rate=16000",
            "--channels=1",
            "--latency-msec=100",
            "--process-time-msec=100",
        ])
        .stdout(Stdio::piped());
    command
}

fn terminate_child(child: &mut Child) {
    let process_id = child.id().to_string();
    let _ = Command::new("kill").args(["-TERM", &process_id]).status();
    let _ = child.wait();
}

fn command_error(fallback: &str, stderr: &[u8]) -> String {
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    if message.is_empty() {
        return fallback.to_owned();
    }
    format!("{fallback}: {message}")
}
