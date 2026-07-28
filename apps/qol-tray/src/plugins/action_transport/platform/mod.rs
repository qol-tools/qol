use super::DaemonActionDispatch;
use std::path::Path;
use std::time::Duration;

pub(super) trait ActionTransportPlatform {
    fn default_io_timeout() -> Duration;
    fn dispatch_action(
        endpoint: &Path,
        action_id: &str,
        input: &serde_json::Value,
        timeout: Duration,
    ) -> DaemonActionDispatch;
    fn can_connect(endpoint: &Path) -> bool;
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

pub(super) fn dispatch_action(
    endpoint: &Path,
    action_id: &str,
    input: &serde_json::Value,
    timeout: Duration,
) -> DaemonActionDispatch {
    Platform::dispatch_action(endpoint, action_id, input, timeout)
}

pub(super) fn can_connect(endpoint: &Path) -> bool {
    Platform::can_connect(endpoint)
}
