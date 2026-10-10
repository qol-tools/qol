use anyhow::{bail, Context, Result};
use qol_platform::windows_path::{parent, same_path, within};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::installer) enum UninstallPlan {
    RemoveRoot(PathBuf),
    RemoveAfterExit(PathBuf),
}

pub(in crate::installer) fn plan(
    binary: &Path,
    install_dir: &Path,
    current_exe: &Path,
) -> Result<UninstallPlan> {
    let binary_dir = parent(binary).unwrap_or_else(|| binary.to_path_buf());
    if !same_path(&binary_dir, install_dir) {
        bail!(
            "refusing to remove {}: it is not the QoL Tray install directory {}",
            binary.display(),
            install_dir.display()
        );
    }
    let root = parent(install_dir).context("Install directory has no parent")?;
    if within(current_exe, &root) {
        return Ok(UninstallPlan::RemoveAfterExit(root));
    }
    Ok(UninstallPlan::RemoveRoot(root))
}

#[cfg(test)]
mod tests {
    use super::*;

    const INSTALL_DIR: &str = r"C:\Users\x\AppData\Local\Programs\qol-tray\bin";
    const ROOT: &str = r"C:\Users\x\AppData\Local\Programs\qol-tray";
    const BINARY: &str = r"C:\Users\x\AppData\Local\Programs\qol-tray\bin\qol-tray.exe";

    #[test]
    fn uninstall_plan_deletes_the_root_or_defers_while_running_inside_it() {
        let cases = [
            (
                BINARY,
                r"C:\Users\x\Downloads\qol-tray-install.exe",
                Some(UninstallPlan::RemoveRoot(PathBuf::from(ROOT))),
            ),
            (
                BINARY,
                r"C:\Users\x\AppData\Local\Programs\qol-tray-old\bin\qol-tray-install.exe",
                Some(UninstallPlan::RemoveRoot(PathBuf::from(ROOT))),
            ),
            (
                BINARY,
                r"C:\Users\x\AppData\Local\Programs\qol-tray\bin\qol-tray-install.exe",
                Some(UninstallPlan::RemoveAfterExit(PathBuf::from(ROOT))),
            ),
            (
                BINARY,
                r"\\?\C:\USERS\X\AppData\Local\Programs\qol-tray\bin\qol-tray-install.exe",
                Some(UninstallPlan::RemoveAfterExit(PathBuf::from(ROOT))),
            ),
            (
                r"c:/users/x/appdata/local/programs/qol-tray/bin/qol-tray.exe",
                r"C:\Users\x\AppData\Local\Programs\qol-tray\uninstall.exe",
                Some(UninstallPlan::RemoveAfterExit(PathBuf::from(ROOT))),
            ),
            (
                r"C:\Users\x\Downloads\qol-tray.exe",
                r"C:\Users\x\Downloads\qol-tray-install.exe",
                None,
            ),
            (
                r"C:\Users\x\AppData\Local\Programs\qol-tray\bin\nested\qol-tray.exe",
                r"C:\Users\x\Downloads\qol-tray-install.exe",
                None,
            ),
            (
                r"C:\Users\x\AppData\Local\Programs\qol-tray\bin-old\qol-tray.exe",
                r"C:\Users\x\Downloads\qol-tray-install.exe",
                None,
            ),
        ];
        for (binary, current_exe, expected) in cases {
            let planned = plan(
                Path::new(binary),
                Path::new(INSTALL_DIR),
                Path::new(current_exe),
            );
            assert_eq!(
                planned.ok(),
                expected,
                "binary: {binary} current_exe: {current_exe}"
            );
        }
    }

    #[test]
    fn uninstall_plan_never_targets_a_drive_root() {
        let planned = plan(
            Path::new(r"C:\bin\qol-tray.exe"),
            Path::new(r"C:\bin"),
            Path::new(r"C:\Users\x\Downloads\qol-tray-install.exe"),
        );
        assert!(planned.is_err(), "{planned:?}");
    }
}
