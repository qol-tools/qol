#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod support;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::{control, current_support};
#[cfg(target_os = "linux")]
pub(crate) use linux::{control, current_support};
#[cfg(target_os = "macos")]
pub(crate) use macos::{control, current_support};
pub(crate) use support::PlatformSupport;
#[cfg(target_os = "windows")]
pub(crate) use windows::{control, current_support};
