#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod tool;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::read;
#[cfg(target_os = "linux")]
pub(crate) use linux::read;
#[cfg(target_os = "macos")]
pub(crate) use macos::read;
#[cfg(target_os = "windows")]
pub(crate) use windows::read;
