use std::io;
use std::path::{Path, PathBuf};

use super::ConnectResult;

pub(super) fn connect(_path: &Path) -> ConnectResult {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "qol runtime client transport is unavailable on this platform",
    ))
}

pub(in crate::client) fn fallback_state_socket() -> Option<PathBuf> {
    None
}
