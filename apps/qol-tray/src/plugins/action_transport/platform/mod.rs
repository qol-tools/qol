#[cfg(unix)]
mod unix_common;

#[cfg(not(any(unix, target_os = "windows")))]
mod unsupported;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(unix)]
use unix_common as imp;
#[cfg(not(any(unix, target_os = "windows")))]
use unsupported as imp;
#[cfg(target_os = "windows")]
use windows as imp;

use super::DaemonActionDispatch;
use std::path::Path;

pub(super) fn dispatch_action(endpoint: &Path, action_id: &str) -> DaemonActionDispatch {
    imp::dispatch_action(endpoint, action_id)
}
