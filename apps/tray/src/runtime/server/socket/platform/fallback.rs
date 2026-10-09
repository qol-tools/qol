use qol_runtime::local_ipc::LocalStream;

pub(super) fn peer_is_alive(_stream: &LocalStream) -> bool {
    true
}

pub(super) fn register_lifeline_for_exec_handoff(_stream: &LocalStream) {}

pub(super) fn unregister_lifeline_for_exec_handoff(_stream: &LocalStream) {}
