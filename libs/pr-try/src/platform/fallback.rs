use anyhow::Result;
use std::path::{Path, PathBuf};

pub(crate) fn tray_binary(tray_dir: &Path) -> Result<PathBuf> {
    Ok(tray_dir.join(crate::TRAY_BINARY))
}

pub(crate) fn prepare_tray(_binary: &Path) -> Result<()> {
    Ok(())
}

pub(crate) fn prepare_plugin(_binary: &Path, _bin_name: &str) -> Result<()> {
    Ok(())
}
