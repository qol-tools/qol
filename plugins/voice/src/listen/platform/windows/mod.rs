use std::sync::atomic::AtomicU64;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::time::Instant;

use crate::audio::AudioFrame;

use super::super::{
    ActiveAudioInput, AudioInput, AudioInputDevice, AudioInputInfo, AudioInputProbe,
    AudioInputRequest, ListenError, ListenMessage,
};

pub(crate) struct PlatformAudioInput;

impl PlatformAudioInput {
    pub(crate) fn new(_request: AudioInputRequest) -> Self {
        Self
    }
}

impl AudioInput for PlatformAudioInput {
    fn info(&self) -> Result<AudioInputInfo, ListenError> {
        Err(ListenError::InputUnavailable(
            "live capture is not implemented on Windows yet".to_owned(),
        ))
    }

    fn start(
        &self,
        _session_started_at: Instant,
        _frames: SyncSender<AudioFrame>,
        _dropped: Arc<AtomicU64>,
        _events: Sender<ListenMessage>,
    ) -> Result<Box<dyn ActiveAudioInput>, ListenError> {
        Err(ListenError::InputUnavailable(
            "live capture is not implemented on Windows yet".to_owned(),
        ))
    }
}

pub(crate) fn audio_input_devices() -> Result<Vec<AudioInputDevice>, ListenError> {
    Err(ListenError::InputUnavailable(
        "audio input discovery is not implemented on Windows yet".to_owned(),
    ))
}

pub(crate) fn verify_audio_input() -> Result<(), ListenError> {
    Err(ListenError::InputUnavailable(
        "live capture is not implemented on Windows yet".to_owned(),
    ))
}

pub(crate) fn probe_audio_input(
    _input: AudioInputRequest,
    _duration_ms: u64,
) -> Result<AudioInputProbe, ListenError> {
    Err(ListenError::InputUnavailable(
        "audio input diagnostics are not implemented on Windows yet".to_owned(),
    ))
}
use crossbeam_channel::Sender;
