use anyhow::{anyhow, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::file_io;

const ACTIVE_INSTALL_ID_FILE: &str = qol_config::ACTIVE_INSTALL_ID_FILE;

pub(super) fn marker_path_for(current_exe: &Path) -> Result<PathBuf> {
    crate::paths::install_marker::existing_marker_path(current_exe)
        .or_else(|| crate::paths::install_marker::marker_path(current_exe))
        .ok_or_else(|| {
            anyhow!(
                "current executable has no parent directory: {}",
                current_exe.display()
            )
        })
}

pub(super) fn active_install_id_path() -> Result<PathBuf> {
    let base = qol_config::data_dir().context("could not determine local data directory")?;
    Ok(base.join(ACTIVE_INSTALL_ID_FILE))
}

pub(super) fn read_install_id_file(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let trimmed = content.trim();
    if qol_config::valid_install_id(trimmed) {
        return Some(trimmed.to_string());
    }
    None
}

pub(super) fn write_install_id_file(path: &Path, install_id: &str) -> Result<()> {
    if !qol_config::valid_install_id(install_id) {
        anyhow::bail!("invalid install id");
    }

    file_io::ensure_parent_dir(path)?;
    fs::write(path, format!("{}\n", install_id))
        .with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_install_id_path_nests_namespace_and_marker_file() {
        let path = active_install_id_path().expect("data dir resolves in test env");
        assert!(
            path.ends_with(ACTIVE_INSTALL_ID_FILE),
            "expected {ACTIVE_INSTALL_ID_FILE} leaf, got {path:?}"
        );
        let parent = path.parent().expect("active-install-id has a parent");
        assert!(
            parent.ends_with(qol_config::NAMESPACE),
            "expected parent under {} namespace, got {parent:?}",
            qol_config::NAMESPACE
        );
    }
}
