use std::io;
use std::path::Path;

pub type LocalListener = uds_windows::UnixListener;
pub type LocalStream = uds_windows::UnixStream;

/// `sun_path` is 108 bytes on Windows as on Linux, minus the NUL.
pub(super) const MAX_SOCKET_PATH_BYTES: usize = 107;

pub(super) fn bind_listener(path: &Path) -> io::Result<LocalListener> {
    LocalListener::bind(path)
}

/// Sockets live under the per-user profile, whose ACLs already keep other
/// users out, and Windows exposes no peer credentials for AF_UNIX.
pub(super) fn authorize_peer(_stream: &LocalStream) -> io::Result<()> {
    Ok(())
}
