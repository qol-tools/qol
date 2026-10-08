use std::io;
use std::path::PathBuf;

pub(crate) const BINARY_FILENAME: &str = "qol-tray";

pub(crate) fn install_dir() -> io::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| io::Error::other("Could not determine home directory"))?;
    Ok(home.join(".local").join("bin"))
}
