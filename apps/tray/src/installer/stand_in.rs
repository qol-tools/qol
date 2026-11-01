use std::path::{Path, PathBuf};

use super::boot_environment::{existing_production_binary, paths_equal_canonicalized};

pub fn recorded_binary() -> std::io::Result<PathBuf> {
    let current = current_executable()?;
    Ok(pick_recorded(current, existing_production_binary()))
}

pub fn is_stand_in() -> bool {
    let Ok(current) = current_executable() else {
        return false;
    };
    stands_in(&current, existing_production_binary().as_deref())
}

fn current_executable() -> std::io::Result<PathBuf> {
    std::env::current_exe().map(qol_conventions::artifact::normalized_executable)
}

fn pick_recorded(current: PathBuf, installed: Option<PathBuf>) -> PathBuf {
    installed.unwrap_or(current)
}

fn stands_in(current: &Path, installed: Option<&Path>) -> bool {
    installed.is_some_and(|installed| !paths_equal_canonicalized(current, installed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_outside_the_install_records_the_installed_tray() {
        let installed = PathBuf::from("/home/u/.local/bin/qol-tray");
        let try_build = PathBuf::from("/home/u/.cache/qol-tray/pr-try/425-659a8dcd4/tray/qol-tray");
        assert_eq!(
            pick_recorded(try_build.clone(), Some(installed.clone())),
            installed
        );
        assert!(stands_in(&try_build, Some(&installed)));
    }

    #[test]
    fn the_installed_tray_records_itself_and_is_no_stand_in() {
        let installed = PathBuf::from("/home/u/.local/bin/qol-tray");
        assert_eq!(
            pick_recorded(installed.clone(), Some(installed.clone())),
            installed
        );
        assert!(!stands_in(&installed, Some(&installed)));
    }

    #[test]
    fn without_an_install_the_running_build_records_itself() {
        let portable = PathBuf::from("/opt/qol/qol-tray");
        assert_eq!(pick_recorded(portable.clone(), None), portable);
        assert!(!stands_in(&portable, None));
    }

    #[test]
    fn a_replaced_installed_binary_still_counts_as_the_install() {
        let root = tempfile::tempdir().unwrap();
        let installed = root.path().join("qol-tray");
        std::fs::write(&installed, b"binary").unwrap();
        let running = qol_conventions::artifact::normalized_executable(PathBuf::from(format!(
            "{} (deleted)",
            installed.display()
        )));
        assert!(!stands_in(&running, Some(&installed)));
    }

    #[test]
    fn the_test_runner_without_an_install_is_no_stand_in() {
        let _guard = crate::test_support::env_lock().blocking_lock();
        let tmp = tempfile::TempDir::new().unwrap();
        let _path_guard = crate::paths::push_test_path_root(tmp.path());
        assert!(!is_stand_in());
        assert_eq!(recorded_binary().unwrap(), current_executable().unwrap());
    }
}
