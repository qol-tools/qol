use std::path::PathBuf;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod local_models;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
use fallback as selected;
#[cfg(target_os = "linux")]
use linux as selected;
#[cfg(target_os = "windows")]
use windows as selected;

pub(super) fn model_cache_dir() -> Option<PathBuf> {
    selected::model_cache_dir()
}

pub(super) fn audio_service_fix() -> &'static str {
    selected::audio_service_fix()
}
