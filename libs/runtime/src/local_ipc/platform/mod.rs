use std::io;
use std::path::Path;

#[cfg(not(any(unix, windows)))]
mod fallback;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(not(any(unix, windows)))]
use fallback as active;
#[cfg(unix)]
use unix as active;
#[cfg(windows)]
use windows as active;

pub use active::{LocalListener, LocalStream};

pub(super) const MAX_SOCKET_PATH_BYTES: usize = active::MAX_SOCKET_PATH_BYTES;

pub(super) fn bind_listener(path: &Path) -> io::Result<LocalListener> {
    active::bind_listener(path)
}

pub(super) fn authorize_peer(stream: &LocalStream) -> io::Result<()> {
    active::authorize_peer(stream)
}
