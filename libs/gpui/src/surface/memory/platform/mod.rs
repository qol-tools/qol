#[cfg(not(any(unix, windows)))]
mod fallback;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
pub(super) use fallback::flush_on_termination;
#[cfg(unix)]
pub(super) use unix::flush_on_termination;
#[cfg(windows)]
pub(super) use windows::flush_on_termination;
