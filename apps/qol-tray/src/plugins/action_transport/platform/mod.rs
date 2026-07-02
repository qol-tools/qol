#[cfg(unix)]
mod unix_common;

use super::DaemonActionDispatch;
use std::path::Path;
use std::time::Duration;

#[cfg(not(any(unix, target_os = "windows")))]
compile_error!(
    "plugins::action_transport::platform::dispatch_action is not implemented for this target OS"
);

#[cfg(unix)]
pub(super) fn default_io_timeout() -> Duration {
    unix_common::DEFAULT_IO_TIMEOUT
}

#[cfg(unix)]
pub(super) fn dispatch_action(
    endpoint: &Path,
    action_id: &str,
    timeout: Duration,
) -> DaemonActionDispatch {
    unix_common::dispatch_action(endpoint, action_id, timeout)
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
    _timeout: Duration,
) -> DaemonActionDispatch {
    DaemonActionDispatch::Unavailable
}

#[cfg(target_os = "windows")]
pub(super) fn can_connect(_endpoint: &Path) -> bool {
    false
}
