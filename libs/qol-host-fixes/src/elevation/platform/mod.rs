#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use fallback::{available, run, spawn};
#[cfg(target_os = "linux")]
pub(super) use linux::{available, run, spawn};
#[cfg(target_os = "macos")]
pub(super) use macos::{available, run, spawn};
#[cfg(target_os = "windows")]
pub(super) use windows::{available, run, spawn};
