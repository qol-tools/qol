#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
pub(super) use fallback::replace_process;
#[cfg(unix)]
pub(super) use unix::replace_process;
