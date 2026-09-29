use std::path::Path;
use std::time::Duration;

mod platform;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonActionDispatch {
    Handled {
        payload: Option<serde_json::Value>,
    },
    Fallback,
    Error(String),
    NotSent,
    OutcomeUnknown,
    NotReady {
        phase: qol_runtime::protocol::ReadinessPhase,
        detail: Option<String>,
    },
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

pub fn dispatch_daemon_action_with_input_and_timeout(
    endpoint: &Path,
    action_id: &str,
    input: &serde_json::Value,
    timeout: Duration,
) -> DaemonActionDispatch {
    dispatch_daemon_action_request(endpoint, action_id, input, timeout)
}

pub fn dispatch_daemon_theme(endpoint: &Path, native: &str, accent: &str) -> DaemonActionDispatch {
    let line = format!("theme {native} {accent}");
    platform::dispatch_action(
        endpoint,
        &line,
        &serde_json::Value::Null,
        platform::default_io_timeout(),
    )
}

fn dispatch_daemon_action_request(
    endpoint: &Path,
    action_id: &str,
    input: &serde_json::Value,
    timeout: Duration,
) -> DaemonActionDispatch {
    if !crate::plugins::manifest::is_valid_action_id(action_id) {
        return DaemonActionDispatch::NotSent;
    }
    platform::dispatch_action(endpoint, action_id, input, timeout)
}

pub fn daemon_listener_reachable(endpoint: &Path) -> bool {
    platform::can_connect(endpoint)
}

#[cfg(test)]
pub(crate) fn default_io_timeout() -> Duration {
    platform::default_io_timeout()
}

pub(crate) fn probe_operation_instance(endpoint: &Path, instance: &str, timeout: Duration) -> bool {
    let DaemonActionDispatch::Handled {
        payload: Some(payload),
    } = dispatch_daemon_action_with_timeout(endpoint, "ping", timeout)
    else {
        return false;
    };
    serde_json::from_value::<qol_runtime::protocol::DaemonOperationCapability>(payload)
        .is_ok_and(|capability| capability.fence_version == 1 && capability.instance == instance)
}

pub(crate) fn dispatch_prepared(
    endpoint: &Path,
    payload: &[u8],
    timeout: Duration,
) -> DaemonActionDispatch {
    platform::dispatch_payload(endpoint, payload, timeout)
}
