use std::path::Path;

#[cfg(any(unix, test))]
mod protocol;

#[cfg(unix)]
mod unix;
#[cfg(not(any(unix, windows)))]
mod unsupported;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as platform;
#[cfg(not(any(unix, windows)))]
use unsupported as platform;
#[cfg(windows)]
use windows as platform;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonActionDispatch {
    Handled,
    Fallback,
    Error(String),
    Unavailable,
}

pub fn dispatch_daemon_action(endpoint: &Path, action_id: &str) -> DaemonActionDispatch {
    if !crate::plugins::manifest::is_valid_action_id(action_id) {
        return DaemonActionDispatch::Fallback;
    }
    platform::dispatch_action(endpoint, action_id)
}
