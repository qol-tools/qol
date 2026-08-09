#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum Observation {
    Unsupported,
    NotLoaded,
    LoadedUnavailable,
    OnDiskUnavailable { loaded: String },
    Matched { loaded: String },
    Mismatch { loaded: String, on_disk: String },
}

#[cfg(all(test, target_os = "linux"))]
pub(super) use linux::bounded_modinfo_version;
#[cfg(target_os = "linux")]
pub(super) use linux::{observe, watch_supported};
#[cfg(target_os = "macos")]
pub(super) use macos::{observe, watch_supported};
#[cfg(target_os = "windows")]
pub(super) use windows::{observe, watch_supported};
