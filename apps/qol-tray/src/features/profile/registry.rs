use anyhow::Result;

use crate::paths;

pub use qol_profile_sync::SyncTarget;

pub fn ensure_profile_dirs_for(name: &str) -> Result<()> {
    let store = super::ProfileScopeStore::new(
        paths::profile_dir()?,
        name.to_string(),
        paths::current_os_subdir().to_string(),
    )?;
    store.ensure_dirs()
}

pub fn load_sync_target() -> Result<Option<SyncTarget>> {
    qol_profile_sync::load_sync_target(&paths::profile_dir()?)
}

pub fn save_sync_target(target: &SyncTarget) -> Result<()> {
    qol_profile_sync::save_sync_target(&paths::profile_dir()?, target)
}

pub fn clear_sync_target() -> Result<()> {
    qol_profile_sync::clear_sync_target(&paths::profile_dir()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh_env() -> (TempDir, paths::TestPathRootGuard) {
        let tmp = TempDir::new().unwrap();
        let guard = paths::push_test_path_root(tmp.path());
        (tmp, guard)
    }

    #[test]
    fn ensure_profile_dirs_for_is_idempotent() {
        let (_tmp, _guard) = fresh_env();
        for _ in 0..3 {
            ensure_profile_dirs_for("default").unwrap();
        }
        let root = paths::profile_dir().unwrap().join("default");
        assert!(root.join("core").is_dir());
    }

    #[test]
    fn sync_target_load_returns_none_when_unset() {
        let (_tmp, _guard) = fresh_env();
        assert!(load_sync_target().unwrap().is_none());
    }

    #[test]
    fn sync_target_save_and_load_roundtrip() {
        let (_tmp, _guard) = fresh_env();
        let target = SyncTarget {
            repo_url: "https://github.com/me/qol-tray-profiles".to_string(),
            auto_created: true,
        };
        save_sync_target(&target).unwrap();
        assert_eq!(load_sync_target().unwrap().as_ref(), Some(&target));
    }

    #[test]
    fn clear_sync_target_removes_file() {
        let (_tmp, _guard) = fresh_env();
        save_sync_target(&SyncTarget {
            repo_url: "x".to_string(),
            auto_created: false,
        })
        .unwrap();
        clear_sync_target().unwrap();
        assert!(load_sync_target().unwrap().is_none());

        clear_sync_target().unwrap();
    }
}
