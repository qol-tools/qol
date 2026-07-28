#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod unix;
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

trait TracePlatformOps {
    fn process_name(&self, pid: &str) -> Option<String>;
    fn initial_monitor_bounds(&self) -> Vec<(i64, i64, i64, i64)>;
}

pub(super) fn process_name(pid: &str) -> Option<String> {
    Platform.process_name(pid)
}

pub(super) fn initial_monitor_bounds() -> Vec<(i64, i64, i64, i64)> {
    Platform.initial_monitor_bounds()
}
