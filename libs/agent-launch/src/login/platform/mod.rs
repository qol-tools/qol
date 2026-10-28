#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
pub(super) use fallback::write_private;
#[cfg(unix)]
pub(super) use unix::write_private;
