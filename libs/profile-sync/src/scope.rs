//! Sync allowlist and mergeable-path rules shared by every sync consumer.
//!
//! Only the manifest, `core/`, `os/<bucket>/`, and `sync/backups/` subtrees
//! of a profile are ever part of the synced profile. The tray's
//! `ProfileScopeStore` and `qol sync` both route their decisions through
//! this module so the allowlist cannot drift between entry points.

use std::path::{Component, Path, PathBuf};

pub const CORE_SUBDIR: &str = "core";
pub const OS_SUBDIR: &str = "os";
pub const DEVICE_SUBDIR: &str = "device";
pub const SYNC_SUBDIR: &str = "sync";
pub const BACKUPS_SUBDIR: &str = "backups";
pub const PLUGIN_CONFIGS_SUBDIR: &str = "plugin-configs";

pub fn current_os_bucket() -> &'static str {
    std::env::consts::OS
}

pub const MANIFEST_FILE: &str = "manifest.json";
pub const PLUGINS_LOCK_FILE: &str = "plugins.lock.json";

/// Same ignore rules as the tray's connect flow, so tracked content stays
/// limited to the sync allowlist plus anything the user adds.
pub const GITIGNORE_CONTENTS: &str = "/active\n/sync.json\n*/device/\n";

/// Whether a relative profile path is part of the synced profile.
pub fn is_sync_allowlisted(rel: &Path) -> bool {
    if rel
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return false;
    }
    let parts: Vec<&str> = rel
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    match parts.as_slice() {
        [".gitignore"] => true,
        [_, file] if *file == MANIFEST_FILE => true,
        [_, scope, _, ..] if *scope == CORE_SUBDIR => true,
        [_, scope, _bucket, _, ..] if *scope == OS_SUBDIR => true,
        [_, top, sub, _, ..] if *top == SYNC_SUBDIR && *sub == BACKUPS_SUBDIR => true,
        _ => false,
    }
}

/// Error from [`device_local_dir`] when a profile name or namespace cannot
/// be safely mapped under the profile root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceLocalDirError {
    /// The profile or namespace was empty, or contained an empty component.
    Empty,
    /// The profile or namespace was absolute.
    Absolute,
    /// The profile or namespace escapes the profile root through `..`.
    ParentDir,
}

impl std::fmt::Display for DeviceLocalDirError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceLocalDirError::Empty => write!(f, "profile or namespace is empty"),
            DeviceLocalDirError::Absolute => {
                write!(f, "profile or namespace must be relative")
            }
            DeviceLocalDirError::ParentDir => {
                write!(f, "profile or namespace must stay under the profile root")
            }
        }
    }
}

impl std::error::Error for DeviceLocalDirError {}

/// Directory under `<profile>/device/` for machine-local state that must
/// never sync. The namespace is validated: it must be relative, contain no
/// `..` component, and have no empty components, so the result always stays
/// under the profile root.
pub fn device_local_dir(profile: &str, namespace: &str) -> Result<PathBuf, DeviceLocalDirError> {
    validate_path(profile)?;
    validate_path(namespace)?;
    Ok(Path::new(profile).join(DEVICE_SUBDIR).join(namespace))
}

fn validate_path(value: &str) -> Result<(), DeviceLocalDirError> {
    if value.is_empty() || value.split(['/', '\\']).any(|p| p.is_empty()) {
        return Err(DeviceLocalDirError::Empty);
    }
    for component in Path::new(value).components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            Component::ParentDir => return Err(DeviceLocalDirError::ParentDir),
            Component::RootDir | Component::Prefix(_) => return Err(DeviceLocalDirError::Absolute),
        }
    }
    Ok(())
}

/// Files that participate in field-level merging: allowlisted JSON outside
/// the `sync` subtree (backups ride along as whole files, never merged).
pub fn mergeable_path(rel: &Path) -> bool {
    is_sync_allowlisted(rel)
        && rel.extension().map(|ext| ext == "json").unwrap_or(false)
        && !rel.components().any(|c| c.as_os_str() == SYNC_SUBDIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_accepts_gitignore_manifest_core_os_bucket_and_sync_backups() {
        let cases = [
            ".gitignore",
            "default/manifest.json",
            "default/core/plugins.lock.json",
            "default/core/plugin-configs/plugin-x.json",
            "default/os/macos/plugin-configs/plugin-keyremap.json",
            "default/os/linux/hotkeys.json",
            "default/sync/backups/20260508-conflict.json",
            "work/core/plugins.lock.json",
        ];
        for raw in cases {
            assert!(is_sync_allowlisted(Path::new(raw)), "must allowlist: {raw}");
        }
    }

    #[test]
    fn allowlist_rejects_device_sync_state_active_marker_and_unknowns() {
        let cases = [
            "default/device/shortcuts.json",
            "default/device/sync/state.json",
            "default/sync/state.json",
            "default/sync/toggles.json",
            "active",
            "sync.json",
            "random.txt",
            "default/random.txt",
            "default/os/macos",
            "default/os",
            "default",
            "../default/core/plugins.lock.json",
            "/default/core/plugins.lock.json",
            "default/../core/plugins.lock.json",
        ];
        for raw in cases {
            assert!(
                !is_sync_allowlisted(Path::new(raw)),
                "must not allowlist: {raw}"
            );
        }
    }

    #[test]
    fn device_local_dir_is_never_sync_allowlisted() {
        let namespaces = ["shortcuts", "window-actions", "a/b/nested"];
        for ns in namespaces {
            let dir = device_local_dir("default", ns).unwrap();
            assert!(!is_sync_allowlisted(&dir), "must reject dir: {dir:?}");
            assert!(
                !is_sync_allowlisted(&dir.join("layout.json")),
                "must reject file: {:?}",
                dir.join("layout.json")
            );
            assert!(
                !is_sync_allowlisted(&dir.join("deep").join("state.json")),
                "must reject nested file: {:?}",
                dir.join("deep").join("state.json")
            );
        }
    }

    #[test]
    fn device_local_dir_round_trips_through_components_and_strings() {
        for (profile, ns) in [("default", "shortcuts"), ("work", "window-actions")] {
            let dir = device_local_dir(profile, ns).unwrap();
            assert_eq!(dir, Path::new(profile).join("device").join(ns));
            let s = dir.to_string_lossy().into_owned();
            let reparsed = Path::new(&s);
            assert_eq!(reparsed, dir, "string form must re-parse to the same path");
            assert!(reparsed
                .components()
                .all(|c| matches!(c, Component::Normal(_))));
            assert_eq!(
                dir.join("layout.json").to_string_lossy(),
                format!("{profile}/device/{ns}/layout.json")
            );
        }
    }

    #[test]
    fn device_local_dir_rejects_unsafe_namespaces() {
        let bad = ["", "..", "../up", "a/../b", "/abs", "a/", "a//b"];
        for ns in bad {
            assert!(
                device_local_dir("default", ns).is_err(),
                "must reject namespace: {ns:?}"
            );
        }
    }

    #[test]
    fn device_local_dir_rejects_unsafe_profiles() {
        let bad = ["", "..", "../up", "/abs", "a/.."];
        for profile in bad {
            assert!(
                device_local_dir(profile, "shortcuts").is_err(),
                "must reject profile: {profile:?}"
            );
        }
    }

    #[test]
    fn device_local_dir_stays_under_profile_root() {
        for (profile, ns) in [("default", "shortcuts"), ("work", "a/b/nested")] {
            let dir = device_local_dir(profile, ns).unwrap();
            assert!(
                dir.starts_with(profile),
                "must stay under profile root: {dir:?}"
            );
        }
    }

    #[test]
    fn mergeable_path_accepts_profile_json_and_excludes_backups() {
        let cases = [
            ("default/manifest.json", true),
            ("default/core/plugin-configs/plugin-a.json", true),
            ("default/os/linux/hotkeys.json", true),
            ("default/sync/backups/20260508-conflict.json", false),
            (".gitignore", false),
            ("default/device/plugin-configs/x.json", false),
        ];
        for (raw, want) in cases {
            assert_eq!(mergeable_path(Path::new(raw)), want, "path: {raw}");
        }
    }
}
