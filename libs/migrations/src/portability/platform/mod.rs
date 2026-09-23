#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
use fallback::Platform;
#[cfg(target_os = "linux")]
use linux::Platform;
#[cfg(target_os = "macos")]
use macos::Platform;
#[cfg(target_os = "windows")]
use windows::Platform;

trait PathPlatform {
    fn path_limit(&self) -> usize;
    fn os_bucket(&self) -> &'static str;
}

pub(super) fn path_limit() -> usize {
    Platform.path_limit()
}

pub(super) fn os_bucket() -> &'static str {
    Platform.os_bucket()
}
