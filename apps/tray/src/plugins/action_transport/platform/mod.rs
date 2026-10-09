use qol_runtime::local_ipc::LocalStream;
use std::path::Path;
use std::time::Duration;

pub(super) trait ActionTransportPlatform {
    fn default_io_timeout() -> Duration;
    fn connect(endpoint: &Path, timeout: Duration) -> Result<LocalStream, ()>;
    /// Runs right before an action is forwarded to a daemon.
    fn before_forward() {}
}

#[cfg(not(any(unix, target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod unix;
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

pub(super) fn default_io_timeout() -> Duration {
    Platform::default_io_timeout()
}

pub(super) fn connect(endpoint: &Path, timeout: Duration) -> Result<LocalStream, ()> {
    Platform::connect(endpoint, timeout)
}

pub(super) fn before_forward() {
    Platform::before_forward();
}
