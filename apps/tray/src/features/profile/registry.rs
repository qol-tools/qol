use anyhow::{bail, Result};
use serde::Serialize;
use std::path::Path;

use crate::daemon::Daemon;
use crate::paths;

pub use qol_profile_sync::SyncTarget;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProfileSummary {
    pub name: String,
    pub active: bool,
    pub plugins: usize,
}

pub fn ensure_profile_dirs_for(name: &str) -> Result<()> {
    scope_store(name)?.ensure_dirs()
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

pub fn list_profiles() -> Result<Vec<String>> {
    let root = paths::profile_dir()?;
    let mut names = Vec::new();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Ok(names);
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if paths::is_safe_path_component(&name) && is_profile_dir(&entry.path()) {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

pub fn profile_summaries() -> Result<Vec<ProfileSummary>> {
    let active = paths::active_profile_name();
    list_profiles()?
        .into_iter()
        .map(|name| {
            let plugins = scope_store(&name)?.core_plugin_config_count();
            Ok(ProfileSummary {
                active: name == active,
                name,
                plugins,
            })
        })
        .collect()
}

pub fn create_profile_from_active(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("Type a name first.");
    }
    if !paths::is_safe_path_component(name) {
        bail!("A profile name uses letters, digits, - and _, and does not start with -.");
    }
    if list_profiles()?.iter().any(|existing| existing == name) {
        bail!("{name} already exists.");
    }
    let target = scope_store(name)?;
    if target.dir().exists() {
        bail!("{name} is already a folder in the profile directory.");
    }
    let source = scope_store(&paths::active_profile_name())?;
    if !source.exists() {
        return target.ensure_dirs();
    }
    source.copy_into(&target)
}

fn scope_store(name: &str) -> Result<super::ProfileScopeStore> {
    super::ProfileScopeStore::new(
        paths::profile_dir()?,
        name.to_string(),
        paths::current_os_subdir().to_string(),
    )
}

fn is_profile_dir(dir: &Path) -> bool {
    dir.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| scope_store(name).ok())
        .is_some_and(|store| store.exists())
}

pub fn switch_active_profile(daemon: &Daemon, name: &str) -> Result<()> {
    if !paths::is_safe_path_component(name) {
        bail!("invalid profile name: {name}");
    }
    let profile_dir = paths::profile_dir()?.join(name);
    if !profile_dir.is_dir() {
        bail!(
            "profile directory does not exist: {}",
            profile_dir.display()
        );
    }
    let marker = paths::active_profile_marker_path()?;
    paths::file_io::ensure_parent_dir(&marker)?;
    std::fs::write(&marker, format!("{name}\n"))?;
    daemon
        .config
        .config_changed(crate::daemon::ConfigKind::Profile);
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
    fn a_new_profile_starts_as_a_copy_of_the_profile_in_use() {
        let (_tmp, _guard) = fresh_env();
        ensure_profile_dirs_for("default").unwrap();
        let active = scope_store("default").unwrap();
        std::fs::write(active.core_plugin_configs_dir().join("a.json"), "{}").unwrap();
        std::fs::create_dir_all(paths::profile_dir().unwrap().join("plugin-configs")).unwrap();

        create_profile_from_active("work").unwrap();

        assert_eq!(
            profile_summaries().unwrap(),
            [
                ProfileSummary {
                    name: "default".to_string(),
                    active: true,
                    plugins: 1,
                },
                ProfileSummary {
                    name: "work".to_string(),
                    active: false,
                    plugins: 1,
                },
            ]
        );
        assert_eq!(
            create_profile_from_active("work").unwrap_err().to_string(),
            "work already exists."
        );
        assert_eq!(
            create_profile_from_active("").unwrap_err().to_string(),
            "Type a name first."
        );
        assert!(create_profile_from_active("-work").is_err());
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
