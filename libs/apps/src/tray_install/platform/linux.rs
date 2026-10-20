use std::io;
use std::path::{Path, PathBuf};

pub(crate) const BINARY_FILENAME: &str = "qol-tray";

pub(crate) fn install_dir() -> io::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| io::Error::other("Could not determine home directory"))?;
    install_dir_for_home(&home)
}

pub(crate) fn install_dir_for_home(home: &Path) -> io::Result<PathBuf> {
    Ok(home.join(".local").join("bin"))
}
