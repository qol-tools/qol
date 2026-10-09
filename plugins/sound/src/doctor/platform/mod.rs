#[cfg(target_os = "linux")]
mod linux;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod server;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::CHECKS;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(crate) use unsupported::CHECKS;
#[cfg(target_os = "windows")]
pub(crate) use windows::CHECKS;
