use std::io;
use std::path::Path;

#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
use fallback as active;
#[cfg(unix)]
use unix as active;

pub use active::{LocalListener, LocalStream};

pub(super) const MAX_SOCKET_PATH_BYTES: usize = active::MAX_SOCKET_PATH_BYTES;

pub(super) fn bind_listener(path: &Path) -> io::Result<LocalListener> {
    active::bind_listener(path)
}

pub(super) fn authorize_peer(stream: &LocalStream) -> io::Result<()> {
    active::authorize_peer(stream)
}
