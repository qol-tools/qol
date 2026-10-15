#[cfg(target_os = "linux")]
mod linux;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(super) use linux::on_stop_request;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(super) use unsupported::on_stop_request;
#[cfg(target_os = "windows")]
pub(super) use windows::on_stop_request;
