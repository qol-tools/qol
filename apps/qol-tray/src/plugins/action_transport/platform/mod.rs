#[cfg(unix)]
mod unix_common;

use super::DaemonActionDispatch;
use std::path::Path;
use std::time::Duration;

#[cfg(unix)]
pub(super) fn default_io_timeout() -> Duration {
    unix_common::DEFAULT_IO_TIMEOUT
}

#[cfg(unix)]
pub(super) fn dispatch_action(
    endpoint: &Path,
    action_id: &str,
    input: &serde_json::Value,
    timeout: Duration,
) -> DaemonActionDispatch {
    unix_common::dispatch_action(endpoint, action_id, input, timeout)
}

#[cfg(unix)]
pub(super) fn can_connect(endpoint: &Path) -> bool {
    unix_common::can_connect(endpoint)
}

#[cfg(target_os = "windows")]
pub(super) fn default_io_timeout() -> Duration {
    Duration::from_secs(10)
}

#[cfg(target_os = "windows")]
pub(super) fn dispatch_action(
    _endpoint: &Path,
    _action_id: &str,
    _input: &serde_json::Value,
    _timeout: Duration,
) -> DaemonActionDispatch {
    DaemonActionDispatch::Unavailable
}

#[cfg(target_os = "windows")]
pub(super) fn can_connect(_endpoint: &Path) -> bool {
    false
}

#[cfg(not(any(unix, target_os = "windows")))]
pub(super) fn default_io_timeout() -> Duration {
    Duration::from_secs(10)
}

#[cfg(not(any(unix, target_os = "windows")))]
pub(super) fn dispatch_action(
    _endpoint: &Path,
    _action_id: &str,
    _input: &serde_json::Value,
    _timeout: Duration,
) -> DaemonActionDispatch {
    DaemonActionDispatch::Unavailable
}

#[cfg(not(any(unix, target_os = "windows")))]
pub(super) fn can_connect(_endpoint: &Path) -> bool {
    false
}
