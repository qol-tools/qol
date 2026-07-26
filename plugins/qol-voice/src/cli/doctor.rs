use anyhow::Result;
use qol_headless::{DoctorCheck, DoctorCheckResult};

use crate::listen::{audio_input_devices, verify_audio_input};

pub(super) fn checks() -> Vec<DoctorCheck> {
    vec![
        DoctorCheck::new(
            "audio_capture",
            "Verify the audio backend exposes at least one microphone.",
            audio_capture_check,
        ),
        DoctorCheck::new(
            "transcription_providers",
            "Verify at least one engine-neutral STT provider is registered.",
            transcription_provider_check,
        ),
        DoctorCheck::new(
            "speech_output",
            "Report the current speech-output capability.",
            speech_output_check,
        ),
    ]
}

fn audio_capture_check() -> Result<DoctorCheckResult> {
    if let Err(error) = verify_audio_input() {
        return Ok(DoctorCheckResult::fail("audio_capture", error.to_string())
            .with_fix("verify PipeWire or PulseAudio is running and reconnect the microphone"));
    }
    let devices = audio_input_devices()?;
    if devices.is_empty() {
        return Ok(DoctorCheckResult::fail(
            "audio_capture",
            "the audio service exposes no microphone sources",
        )
        .with_fix("connect or enable a microphone input"));
    }
    let default = devices
        .iter()
        .find(|device| device.is_default)
        .map(|device| device.label.as_str())
        .unwrap_or("not identified");
    Ok(DoctorCheckResult::ok(
        "audio_capture",
        format!(
            "{} microphone source(s) available; default: {default}",
            devices.len()
        ),
    ))
}

fn transcription_provider_check() -> Result<DoctorCheckResult> {
    let providers = crate::transcribe::transcriber_descriptors().collect::<Vec<_>>();
    if providers.is_empty() {
        return Ok(DoctorCheckResult::fail(
            "transcription_providers",
            "no speech-recognition providers are registered",
        ));
    }
    Ok(DoctorCheckResult::ok(
        "transcription_providers",
        format!(
            "{} provider(s) registered: {}",
            providers.len(),
            providers
                .iter()
                .map(|provider| provider.id)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    ))
}

fn speech_output_check() -> Result<DoctorCheckResult> {
    Ok(DoctorCheckResult::warn(
        "speech_output",
        "TTS playback is outside this MVP; STT and turn coordination are active",
    ))
}
