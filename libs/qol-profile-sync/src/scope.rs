//! Sync allowlist and mergeable-path rules shared by every sync consumer.
//!
//! Only the manifest, `core/`, `os/<bucket>/`, and `sync/backups/` subtrees
//! of a profile are ever part of the synced profile. The tray's
//! `ProfileScopeStore` and `qol sync` both route their decisions through
//! this module so the allowlist cannot drift between entry points.

use std::path::{Component, Path};

pub const CORE_SUBDIR: &str = "core";
pub const OS_SUBDIR: &str = "os";
pub const DEVICE_SUBDIR: &str = "device";
pub const SYNC_SUBDIR: &str = "sync";
pub const BACKUPS_SUBDIR: &str = "backups";
pub const PLUGIN_CONFIGS_SUBDIR: &str = "plugin-configs";

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
