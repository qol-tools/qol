use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::file_io;
use crate::paths;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncTarget {
    pub repo_url: String,
    #[serde(default)]
    pub auto_created: bool,
}

pub fn list_profile_names() -> Result<Vec<String>> {
    let dir = paths::profile_dir()?;
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = std::fs::read_dir(&dir)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| paths::is_safe_path_component(name))
        .collect();
    names.sort();
    Ok(names)
}

pub fn create_profile(name: &str) -> Result<()> {
    if !paths::is_safe_path_component(name) {
        return Err(anyhow!("invalid profile name: {name:?}"));
    }
    if list_profile_names()?
        .iter()
        .any(|existing| existing == name)
    {
        return Err(anyhow!("profile {name:?} already exists"));
    }
    ensure_profile_dirs_for(name)
}

pub fn delete_profile(name: &str) -> Result<()> {
    if !paths::is_safe_path_component(name) {
        return Err(anyhow!("invalid profile name: {name:?}"));
    }
    let dir = paths::profile_dir()?.join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

pub fn ensure_profile_dirs_for(name: &str) -> Result<()> {
    let store = super::ProfileScopeStore::new(
        paths::profile_dir()?,
        name.to_string(),
        paths::current_os_subdir().to_string(),
    )?;
    store.ensure_dirs()
}

pub fn load_sync_target() -> Result<Option<SyncTarget>> {
    let path = paths::profile_sync_config_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

pub fn save_sync_target(target: &SyncTarget) -> Result<()> {
    let path = paths::profile_sync_config_path()?;
    file_io::ensure_parent_dir(&path)?;
    file_io::write_pretty_json(&path, target)
}

pub fn clear_sync_target() -> Result<()> {
    let path = paths::profile_sync_config_path()?;
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
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
    fn list_profile_names_returns_empty_when_dir_absent() {
        let (_tmp, _guard) = fresh_env();
        assert_eq!(list_profile_names().unwrap(), Vec::<String>::new());
    }

    #[test]
    fn list_profile_names_returns_subdirs_sorted() {
        let (_tmp, _guard) = fresh_env();
        for name in ["work", "default", "personal"] {
            ensure_profile_dirs_for(name).unwrap();
        }
        assert_eq!(
            list_profile_names().unwrap(),
            vec!["default", "personal", "work"]
        );
    }

    #[test]
    fn list_profile_names_skips_invalid_entries() {
        let (_tmp, _guard) = fresh_env();
        ensure_profile_dirs_for("work").unwrap();
        let profile_dir = paths::profile_dir().unwrap();
        std::fs::create_dir_all(profile_dir.join(".hidden")).unwrap();
        std::fs::create_dir_all(profile_dir.join("with space")).unwrap();
        std::fs::write(profile_dir.join("active"), b"work\n").unwrap();
        std::fs::write(profile_dir.join("sync.json"), b"{}").unwrap();

        assert_eq!(list_profile_names().unwrap(), vec!["work"]);
    }

    #[test]
    fn create_profile_materializes_full_tree() {
        let (_tmp, _guard) = fresh_env();
        create_profile("work").unwrap();
        let root = paths::profile_dir().unwrap().join("work");
        let expected = [
            root.join("core"),
            root.join("core").join("plugin-configs"),
            root.join("os").join(paths::current_os_subdir()),
            root.join("device"),
        ];
        for path in expected {
            assert!(path.is_dir(), "{} should exist", path.display());
        }
    }

    #[test]
    fn create_profile_rejects_invalid_or_duplicate() {
        let (_tmp, _guard) = fresh_env();
        let invalid = ["", "../escape", "with space", "-leading", "with/slash"];
        for name in invalid {
            assert!(create_profile(name).is_err(), "should reject {name:?}");
        }

        create_profile("work").unwrap();
        assert!(create_profile("work").is_err(), "duplicate must fail");
    }

    #[test]
    fn delete_profile_removes_tree_and_is_idempotent() {
        let (_tmp, _guard) = fresh_env();
        create_profile("work").unwrap();
        let root = paths::profile_dir().unwrap().join("work");
        assert!(root.is_dir());

        delete_profile("work").unwrap();
        assert!(!root.exists());

        delete_profile("work").unwrap();
        assert!(!root.exists());
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
