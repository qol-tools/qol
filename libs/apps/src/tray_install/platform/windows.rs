use std::io;
use std::path::{Path, PathBuf};

pub(crate) const BINARY_FILENAME: &str = "qol-tray.exe";

pub(crate) fn install_dir() -> io::Result<PathBuf> {
    let local_data = dirs::data_local_dir()
        .ok_or_else(|| io::Error::other("Could not determine local data directory"))?;
    Ok(install_dir_in(&local_data))
}

pub(crate) fn install_dir_for_home(home: &Path) -> io::Result<PathBuf> {
    Ok(install_dir_in(&home.join("AppData").join("Local")))
}

fn install_dir_in(local_data: &Path) -> PathBuf {
    local_data.join("Programs").join("qol-tray").join("bin")
}
