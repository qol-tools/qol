use std::io;
use std::path::{Path, PathBuf};

pub(crate) const BINARY_FILENAME: &str = "qol-tray";

pub(crate) fn install_dir() -> io::Result<PathBuf> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "qol-tray install directory is not defined on this OS",
    ))
}

pub(crate) fn install_dir_for_home(_home: &Path) -> io::Result<PathBuf> {
    install_dir()
}
