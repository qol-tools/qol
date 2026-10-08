use std::io;
use std::path::PathBuf;

pub(crate) const BINARY_FILENAME: &str = "qol-tray";

pub(crate) fn install_dir() -> io::Result<PathBuf> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "qol-tray install directory is not defined on this OS",
    ))
}
