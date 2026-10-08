use std::io;
use std::path::PathBuf;

pub(crate) const BINARY_FILENAME: &str = "qol-tray.exe";

pub(crate) fn install_dir() -> io::Result<PathBuf> {
    let local_data = dirs::data_local_dir()
        .ok_or_else(|| io::Error::other("Could not determine local data directory"))?;
    Ok(local_data.join("Programs").join("qol-tray").join("bin"))
}
