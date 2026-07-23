#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(super) use linux::{loaded_version, notify_mismatch, on_disk_version, watch_supported};
#[cfg(target_os = "macos")]
pub(super) use macos::{loaded_version, notify_mismatch, on_disk_version, watch_supported};
#[cfg(target_os = "windows")]
pub(super) use windows::{loaded_version, notify_mismatch, on_disk_version, watch_supported};
