use qol_runtime::local_ipc::LocalStream;

#[cfg(not(any(unix, windows)))]
mod fallback;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
use fallback as active;
#[cfg(unix)]
use unix as active;
#[cfg(windows)]
use windows as active;

/// Whether the client of a held-open connection is still there, probed
/// without consuming what it sent.
pub(super) fn peer_is_alive(stream: &LocalStream) -> bool {
    active::peer_is_alive(stream)
}

pub(super) fn register_lifeline_for_exec_handoff(stream: &LocalStream) {
    active::register_lifeline_for_exec_handoff(stream);
}

pub(super) fn unregister_lifeline_for_exec_handoff(stream: &LocalStream) {
    active::unregister_lifeline_for_exec_handoff(stream);
}
