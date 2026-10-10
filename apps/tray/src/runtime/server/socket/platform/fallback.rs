use qol_runtime::local_ipc::LocalStream;

pub(crate) fn peer_is_alive(_stream: &LocalStream) -> bool {
    true
}

pub(crate) fn register_lifeline_for_exec_handoff(_stream: &LocalStream) {}

pub(crate) fn unregister_lifeline_for_exec_handoff(_stream: &LocalStream) {}
