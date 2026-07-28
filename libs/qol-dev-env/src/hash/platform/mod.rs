#[cfg(not(any(unix, windows)))]
mod fallback;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
pub(super) use fallback::{file_identity, FileIdentity};
#[cfg(unix)]
pub(super) use unix::{file_identity, FileIdentity};
#[cfg(windows)]
pub(super) use windows::{file_identity, FileIdentity};
