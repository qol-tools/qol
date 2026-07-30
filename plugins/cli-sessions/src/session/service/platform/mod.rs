#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
pub(super) use fallback::process_snapshot;
#[cfg(unix)]
pub(super) use unix::process_snapshot;
