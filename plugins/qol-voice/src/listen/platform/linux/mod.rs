use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::audio::{AudioEncoding, AudioFormat, AudioFrame};

use super::super::{
    ActiveAudioInput, AudioInput, AudioInputInfo, AudioInputRequest, ListenError, ListenMessage,
};

mod diagnostics;

pub(crate) use diagnostics::{audio_input_devices, probe_audio_input, verify_audio_input};

const SAMPLE_RATE: u32 = 16_000;
const CHANNELS: u16 = 1;
const BUFFER_BYTES: usize = 3_200;
const CAPTURE_START_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) struct PlatformAudioInput {
    requested_device: Option<String>,
}

struct PulseAudioInput {
    child: Mutex<Child>,
    reader: Option<JoinHandle<()>>,
    stopping: Arc<AtomicBool>,
}

impl ActiveAudioInput for PulseAudioInput {}

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

impl Drop for PulseAudioInput {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        let Ok(mut child) = self.child.lock() else {
            return;
        };
        terminate_child(&mut child);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

impl AudioInput for PlatformAudioInput {
    fn info(&self) -> Result<AudioInputInfo, ListenError> {
        Ok(AudioInputInfo {
            device_name: self.device_name()?,
            format: AudioFormat {
                sample_rate: SAMPLE_RATE,
                channels: CHANNELS,
                encoding: AudioEncoding::PcmS16Le,
            },
        })
    }

    fn start(
        &self,
        session_started_at: Instant,
        frames: SyncSender<AudioFrame>,
        dropped: Arc<std::sync::atomic::AtomicU64>,
        events: Sender<ListenMessage>,
    ) -> Result<Box<dyn ActiveAudioInput>, ListenError> {
        let device_name = self.device_name()?;
        let mut child = capture_command(&device_name).spawn().map_err(|error| {
            ListenError::InputUnavailable(format!(
                "could not start parec: {error}; install PulseAudio utilities"
            ))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            ListenError::InputUnavailable("parec did not provide an audio stream".to_owned())
        })?;
        let stopping = Arc::new(AtomicBool::new(false));
        let reader_stopping = stopping.clone();
        let (capture_started, capture_start) = mpsc::sync_channel(1);
        let reader = thread::spawn(move || {
            capture_audio(
                stdout,
                session_started_at,
                frames,
                dropped,
                events,
                reader_stopping,
                capture_started,
            );
        });
        let input = PulseAudioInput {
            child: Mutex::new(child),
            reader: Some(reader),
            stopping,
        };
        wait_for_capture_start(&capture_start, &device_name)?;
        Ok(Box::new(input))
    }
}

fn capture_audio<R>(
    mut audio: R,
    session_started_at: Instant,
    frames: SyncSender<AudioFrame>,
    dropped: Arc<std::sync::atomic::AtomicU64>,
    events: Sender<ListenMessage>,
    stopping: Arc<AtomicBool>,
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
                let _ = events.send(Err(ListenError::CaptureFailed(message)));
            }
            return;
        }
        if let Some(started) = capture_started.take() {
            if started.send(Ok(())).is_err() {
                return;
            }
        }
        let observed_at_ms =
            u64::try_from(session_started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        let frame = AudioFrame {
            observed_at_ms,
            pcm: buffer.to_vec(),
        };
        match frames.try_send(frame) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                dropped.fetch_add(1, Ordering::Relaxed);
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

fn default_source_name() -> Result<String, ListenError> {
    if let Some(source) = std::env::var_os("PULSE_SOURCE") {
        let source = source.to_string_lossy().trim().to_owned();
        if !source.is_empty() {
            return Ok(source);
        }
    }
    let output = Command::new("pactl")
        .arg("get-default-source")
        .output()
        .map_err(|error| {
            ListenError::InputUnavailable(format!(
                "could not run pactl: {error}; install PulseAudio utilities"
            ))
        })?;
    if !output.status.success() {
        return Err(ListenError::InputUnavailable(command_error(
            "pactl could not resolve the default source",
            &output.stderr,
        )));
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if name.is_empty() {
        return Err(ListenError::NoInputDevice);
    }
    Ok(name)
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
use crossbeam_channel::Sender;
