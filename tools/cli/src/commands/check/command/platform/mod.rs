#[cfg(not(any(unix, windows)))]
mod fallback;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
pub(super) use fallback::{fallback_alive, fallback_force_stop, fallback_request_stop};
#[cfg(unix)]
pub(super) use unix::{fallback_alive, fallback_force_stop, fallback_request_stop};
#[cfg(windows)]
pub(super) use windows::{fallback_alive, fallback_force_stop, fallback_request_stop};
