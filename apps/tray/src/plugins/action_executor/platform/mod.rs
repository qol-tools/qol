pub(super) trait ActionProcessPlatform {
    fn track_desktop_state_pid(pid: u32);
    fn untrack_desktop_state_pid(pid: u32);
}

#[cfg(not(any(unix, target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
mod unix_fallback;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(unix, target_os = "windows")))]
use fallback::Platform;
#[cfg(target_os = "linux")]
use linux::Platform;
#[cfg(target_os = "macos")]
use macos::Platform;
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
use unix_fallback::Platform;
#[cfg(target_os = "windows")]
use windows::Platform;

pub(super) fn track_desktop_state_pid(pid: u32) {
    Platform::track_desktop_state_pid(pid);
}

pub(super) fn untrack_desktop_state_pid(pid: u32) {
    Platform::untrack_desktop_state_pid(pid);
}
