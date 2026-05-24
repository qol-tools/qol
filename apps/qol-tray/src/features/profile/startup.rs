use anyhow::Result;
use std::path::Path;

pub(crate) fn run_startup_cleanup(_config_dir: &Path) -> Result<()> {
    let name = crate::paths::active_profile_name();
    crate::features::profile::registry::ensure_profile_dirs_for(&name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn run_startup_cleanup_bootstraps_active_profile_dirs() {
        let tmp = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(tmp.path());

        run_startup_cleanup(tmp.path()).unwrap();

        let root = crate::paths::profile_dir().unwrap();
        let active = crate::paths::active_profile_name();
        let profile_root = root.join(&active);
        assert!(profile_root.join("core").is_dir());
        assert!(profile_root
            .join("os")
            .join(crate::paths::current_os_subdir())
            .is_dir());
        assert!(profile_root.join("device").is_dir());
    }

    #[test]
    fn run_startup_cleanup_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(tmp.path());

        for _ in 0..3 {
            run_startup_cleanup(tmp.path()).unwrap();
        }
        let active = crate::paths::active_profile_name();
        let core = crate::paths::profile_dir()
            .unwrap()
            .join(active)
            .join("core");
        assert!(core.is_dir());
    }
}
