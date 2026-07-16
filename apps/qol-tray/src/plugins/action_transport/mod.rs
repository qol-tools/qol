use std::path::Path;
use std::time::Duration;

#[cfg(any(unix, test))]
mod protocol;

mod platform;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonActionDispatch {
    Handled { payload: Option<serde_json::Value> },
    Fallback,
    Error(String),
    Unavailable,
}

pub fn dispatch_daemon_action(endpoint: &Path, action_id: &str) -> DaemonActionDispatch {
    dispatch_daemon_action_with_timeout(endpoint, action_id, platform::default_io_timeout())
}

pub fn dispatch_daemon_action_with_input(
    endpoint: &Path,
    action_id: &str,
    input: &serde_json::Value,
) -> DaemonActionDispatch {
    dispatch_daemon_action_request(endpoint, action_id, input, platform::default_io_timeout())
}

pub fn dispatch_daemon_action_with_timeout(
    endpoint: &Path,
    action_id: &str,
    timeout: Duration,
) -> DaemonActionDispatch {
    dispatch_daemon_action_request(endpoint, action_id, &serde_json::Value::Null, timeout)
}

fn dispatch_daemon_action_request(
    endpoint: &Path,
    action_id: &str,
    input: &serde_json::Value,
    timeout: Duration,
) -> DaemonActionDispatch {
    if !crate::plugins::manifest::is_valid_action_id(action_id) {
        return DaemonActionDispatch::Fallback;
    }
    platform::dispatch_action(endpoint, action_id, input, timeout)
}

pub fn daemon_listener_reachable(endpoint: &Path) -> bool {
    platform::can_connect(endpoint)
}
