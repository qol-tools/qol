use std::path::Path;

#[cfg(any(unix, test))]
mod protocol;

mod platform;

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
