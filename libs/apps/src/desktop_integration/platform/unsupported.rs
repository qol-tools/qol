use std::io;
use std::path::{Path, PathBuf};

use super::super::OpenHandler;

fn unavailable<T>() -> io::Result<T> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "choosing an app to open with is not available on this system",
    ))
}

pub(in super::super) fn open_with_supported() -> bool {
    false
}

pub(in super::super) fn file_type(_path: &Path) -> io::Result<String> {
    unavailable()
}

pub(in super::super) fn open_handlers(_path: &Path) -> io::Result<Vec<OpenHandler>> {
    unavailable()
}

pub(in super::super) fn open_with_handler(_handler: &str, _paths: &[PathBuf]) -> io::Result<()> {
    unavailable()
}
