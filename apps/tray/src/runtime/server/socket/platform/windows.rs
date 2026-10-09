use qol_runtime::local_ipc::LocalStream;

/// Windows has no non-consuming peek on these sockets without raw Winsock
/// calls, so a gone peer is noticed when the next event write fails.
pub(super) fn peer_is_alive(_stream: &LocalStream) -> bool {
    true
}

/// A Windows restart spawns a fresh tray, so no connection is handed across.
pub(super) fn register_lifeline_for_exec_handoff(_stream: &LocalStream) {}

pub(super) fn unregister_lifeline_for_exec_handoff(_stream: &LocalStream) {}
