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

pub(super) use active::{
    peer_is_alive, register_lifeline_for_exec_handoff, unregister_lifeline_for_exec_handoff,
};
