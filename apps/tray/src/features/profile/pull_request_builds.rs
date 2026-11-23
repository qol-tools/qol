use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

use super::ProfileScopeStore;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Switch {
    #[serde(default)]
    enabled: bool,
}

pub(crate) fn enabled() -> bool {
    ProfileScopeStore::from_active()
        .is_ok_and(|store| enabled_at(&store.pull_request_builds_path()))
}

pub(crate) fn set(enabled: bool) -> Result<()> {
    let store = ProfileScopeStore::from_active()?;
    set_at(&store.pull_request_builds_path(), enabled)
}

fn enabled_at(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Switch>(&text).ok())
        .is_some_and(|switch| switch.enabled)
}

fn set_at(path: &Path, enabled: bool) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no folder", path.display()))?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let body = serde_json::to_vec_pretty(&Switch { enabled })?;
    qol_fs::atomic_write(path, &body).with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_switch_is_off_until_this_device_turns_it_on() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("device").join("pull-request-builds.json");
        assert!(!enabled_at(&path));
        set_at(&path, true).unwrap();
        assert!(enabled_at(&path));
        set_at(&path, false).unwrap();
        assert!(!enabled_at(&path));
        std::fs::write(&path, b"not json").unwrap();
        assert!(!enabled_at(&path));
    }

    #[test]
    fn the_switch_lives_in_the_device_folder_that_never_syncs() {
        let root = tempfile::tempdir().unwrap();
        let _paths = crate::paths::push_test_path_root(root.path());
        let store = ProfileScopeStore::from_active().unwrap();
        assert!(store
            .pull_request_builds_path()
            .starts_with(store.device_dir()));
        assert!(!enabled());
        set(true).unwrap();
        assert!(enabled());
    }
}
