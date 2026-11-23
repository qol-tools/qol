use anyhow::{Context, Result};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

const RUNNABLE: u32 = 0o755;

pub(crate) fn tray_binary(tray_dir: &Path) -> Result<PathBuf> {
    Ok(tray_dir.join(crate::TRAY_BINARY))
}

pub(crate) fn prepare_tray(binary: &Path) -> Result<()> {
    let mode = std::fs::metadata(binary)
        .with_context(|| format!("reading {}", binary.display()))?
        .permissions()
        .mode();
    std::fs::set_permissions(binary, std::fs::Permissions::from_mode(mode | RUNNABLE))
        .with_context(|| format!("making {} runnable", binary.display()))
}

pub(crate) fn prepare_plugin(_binary: &Path, _bin_name: &str) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tray_from_a_zip_becomes_runnable() {
        let root = tempfile::tempdir().unwrap();
        let binary = tray_binary(root.path()).unwrap();
        std::fs::write(&binary, b"tray").unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o644)).unwrap();
        prepare_tray(&binary).unwrap();
        let mode = std::fs::metadata(&binary).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    #[test]
    fn a_missing_tray_says_which_file() {
        let root = tempfile::tempdir().unwrap();
        let error = format!(
            "{:#}",
            prepare_tray(&root.path().join("qol-tray")).unwrap_err()
        );
        assert!(error.contains("qol-tray"), "{error}");
    }
}
