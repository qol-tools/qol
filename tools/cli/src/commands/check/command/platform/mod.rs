#[cfg(not(any(unix, windows)))]
mod fallback;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
pub(super) use fallback::{
    fallback_alive, fallback_force_stop, fallback_request_stop, FALLBACK_BACKEND,
};
#[cfg(unix)]
pub(super) use unix::{
    fallback_alive, fallback_force_stop, fallback_request_stop, FALLBACK_BACKEND,
};
#[cfg(windows)]
pub(super) use windows::{
    fallback_alive, fallback_force_stop, fallback_request_stop, FALLBACK_BACKEND,
};

#[cfg(not(any(unix, windows)))]
pub(in crate::commands::check) use fallback::exit_signal;
#[cfg(unix)]
pub(in crate::commands::check) use unix::exit_signal;
#[cfg(windows)]
pub(in crate::commands::check) use windows::exit_signal;
