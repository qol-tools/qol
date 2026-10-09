#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(super) use fallback::device_id;
#[cfg(target_os = "linux")]
pub(super) use linux::device_id;
#[cfg(target_os = "macos")]
pub(super) use macos::device_id;
