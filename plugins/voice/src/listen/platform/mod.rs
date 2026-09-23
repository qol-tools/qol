#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(super) use linux::{
    audio_input_devices, probe_audio_input, verify_audio_input, PlatformAudioInput,
};
#[cfg(target_os = "macos")]
pub(super) use macos::{
    audio_input_devices, probe_audio_input, verify_audio_input, PlatformAudioInput,
};
#[cfg(target_os = "windows")]
pub(super) use windows::{
    audio_input_devices, probe_audio_input, verify_audio_input, PlatformAudioInput,
};
