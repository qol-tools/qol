#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(super) use linux::platform_supported_check;
#[cfg(target_os = "macos")]
pub(super) use macos::platform_supported_check;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use unsupported::platform_supported_check;
#[cfg(target_os = "windows")]
pub(super) use windows::platform_supported_check;
