#[cfg(unix)]
mod unix_common;

use super::DaemonActionDispatch;
use std::path::Path;

#[cfg(not(any(unix, target_os = "windows")))]
compile_error!(
    "plugins::action_transport::platform::dispatch_action is not implemented for this target OS"
);

#[cfg(unix)]
pub(super) fn dispatch_action(endpoint: &Path, action_id: &str) -> DaemonActionDispatch {
    unix_common::dispatch_action(endpoint, action_id)
}

#[cfg(target_os = "windows")]
pub(super) fn dispatch_action(_endpoint: &Path, _action_id: &str) -> DaemonActionDispatch {
    DaemonActionDispatch::Unavailable
}
