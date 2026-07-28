#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
pub(super) use fallback::source_is_executable;
#[cfg(unix)]
pub(super) use unix::source_is_executable;
