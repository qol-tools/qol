#[cfg(unix)]
mod unix;
#[cfg(not(unix))]
mod unsupported;

#[cfg(unix)]
pub(super) use unix::*;
#[cfg(not(unix))]
pub(super) use unsupported::*;
