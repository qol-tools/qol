use anyhow::Result;
use std::path::{Path, PathBuf};

pub(in crate::commands::pr) fn tray_binary(tray_dir: &Path) -> Result<PathBuf> {
    Ok(tray_dir.join(super::super::tray::BINARY))
}

pub(in crate::commands::pr) fn prepare_tray(_binary: &Path) -> Result<()> {
    Ok(())
}

pub(in crate::commands::pr) fn prepare_plugin(_binary: &Path, _bin_name: &str) -> Result<()> {
    Ok(())
}
