#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
pub(super) use fallback::flush_on_termination;
#[cfg(unix)]
pub(super) use unix::flush_on_termination;
