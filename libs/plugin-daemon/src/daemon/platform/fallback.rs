//! Hosts without fd handoff from qol-tray (Windows and any other non-Unix
//! target): a daemon always binds its own socket path.

use std::io;
use std::path::{Path, PathBuf};

use qol_runtime::local_ipc::LocalListener;

pub(in crate::daemon) fn fallback_socket_dir(_use_tmpdir_env: bool) -> PathBuf {
    std::env::temp_dir()
}

pub(in crate::daemon) fn remove_socket_file(path: impl AsRef<Path>) {
    let _ = std::fs::remove_file(path);
}

pub(in crate::daemon) fn inherited_listener() -> Option<LocalListener> {
    None
}

pub fn restore_cloexec(_fd: i32) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "descriptor handoff is unavailable on this platform",
    ))
}

pub fn inherited_port_fd(_name: &str) -> Option<i32> {
    None
}

pub fn inherited_primary_port_fd() -> Option<i32> {
    None
}
