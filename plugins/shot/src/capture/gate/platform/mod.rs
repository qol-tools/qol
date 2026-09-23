#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod unix_common;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::{try_acquire, CaptureGuard};
#[cfg(target_os = "macos")]
pub(crate) use macos::{try_acquire, CaptureGuard};
#[cfg(target_os = "windows")]
pub(crate) use windows::{try_acquire, CaptureGuard};
