use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::file_io;

const APP_NAME: &str = "qol-tray";
const INSTALL_ID_MARKER_FILE: &str = "qol-tray.install-id";
const ACTIVE_INSTALL_ID_FILE: &str = "active-install-id";

pub(super) fn marker_path_for(current_exe: &Path) -> Result<PathBuf> {
    let Some(parent) = current_exe.parent() else {
        return Err(anyhow!(
            "current executable has no parent directory: {}",
            current_exe.display()
        ));
    };
    Ok(parent.join(INSTALL_ID_MARKER_FILE))
}

pub(super) fn active_install_id_path() -> Result<PathBuf> {
    let base = dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .context("could not determine local data directory")?;
    Ok(base.join(APP_NAME).join(ACTIVE_INSTALL_ID_FILE))
}

pub(super) fn read_install_id_file(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if valid_install_id(trimmed) {
        return Some(trimmed.to_string());
    }
    None
}

pub(super) fn write_install_id_file(path: &Path, install_id: &str) -> Result<()> {
    if !valid_install_id(install_id) {
        anyhow::bail!("invalid install id");
    }

    file_io::ensure_parent_dir(path)?;
    fs::write(path, format!("{}\n", install_id))
        .with_context(|| format!("failed to write {}", path.display()))
}

pub(super) fn canonical_or_original(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn valid_install_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}
