use std::io;
use std::path::Path;

use super::ConnectResult;

pub(super) fn connect(_path: &Path) -> ConnectResult {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "qol runtime client transport is unavailable on this platform",
    ))
}
