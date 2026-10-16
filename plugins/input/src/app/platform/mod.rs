#[cfg(unix)]
mod unix;
#[cfg(not(any(unix, target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(unix)]
pub(super) use unix::on_stop_request;
#[cfg(not(any(unix, target_os = "windows")))]
pub(super) use unsupported::on_stop_request;
#[cfg(target_os = "windows")]
pub(super) use windows::on_stop_request;
