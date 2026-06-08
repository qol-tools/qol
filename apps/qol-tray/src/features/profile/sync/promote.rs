use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

use crate::features::profile::ProfileScopeStore;

pub(super) fn promote_allowlisted_clone(staging: &Path, profile: &Path) -> Result<()> {
    if !staging.is_dir() {
        anyhow::bail!("staging directory missing: {}", staging.display());
    }
    let files = walk_files(staging)?;
    for absolute in files {
        let rel = absolute
            .strip_prefix(staging)
            .with_context(|| format!("strip prefix {}", absolute.display()))?
            .to_path_buf();
        if rel.starts_with(".git") {
            trace_promote_entry(&rel, "skip", "git_dir");
            continue;
        }
        if !ProfileScopeStore::is_sync_allowlisted(&rel) {
            trace_promote_entry(&rel, "skip", "not_allowlisted");
            continue;
        }
        let dst = profile.join(&rel);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create dir {}", parent.display()))?;
        }
        std::fs::copy(&absolute, &dst)
            .with_context(|| format!("copy {} -> {}", absolute.display(), dst.display()))?;
        trace_promote_entry(&rel, "copy", "allowlisted");
    }
    Ok(())
}

pub(super) fn promote_clone_git_dir(staging: &Path, profile: &Path) -> Result<()> {
    let staging_git = staging.join(".git");
    let profile_git = profile.join(".git");
    if !staging_git.is_dir() {
        anyhow::bail!(
            "staging clone did not produce a .git dir at {}",
            staging_git.display()
        );
    }
    if profile_git.exists() {
        std::fs::remove_dir_all(&profile_git)
            .with_context(|| format!("clear {}", profile_git.display()))?;
    }
    if let Some(parent) = profile_git.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("ensure profile dir {}", parent.display()))?;
    }
    std::fs::rename(&staging_git, &profile_git).with_context(|| {
        format!(
            "move {} -> {}",
            staging_git.display(),
            profile_git.display()
        )
    })?;
    Ok(())
}

fn walk_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let metadata =
            std::fs::symlink_metadata(&dir).with_context(|| format!("stat {}", dir.display()))?;
        if metadata.file_type().is_symlink() {
            trace_walk_entry(root, &dir, "skip", "symlink");
            continue;
        }
        if !metadata.is_dir() {
            trace_walk_entry(root, &dir, "skip", "unsupported_entry");
            continue;
        }
        for entry in
            std::fs::read_dir(&dir).with_context(|| format!("read dir {}", dir.display()))?
        {
            let entry = entry?;
            let file_type = entry
                .file_type()
                .with_context(|| format!("read entry type {}", entry.path().display()))?;
            let path = entry.path();
            if file_type.is_symlink() {
                trace_walk_entry(root, &path, "skip", "symlink");
            } else if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                out.push(path);
            } else {
                trace_walk_entry(root, &path, "skip", "unsupported_entry");
            }
        }
    }
    Ok(out)
}

fn trace_promote_entry(rel: &Path, outcome: &str, reason: &str) {
    #[cfg(debug_assertions)]
    {
        qol_runtime::probe!(
            "PROFILE_SYNC_PROMOTE",
            "entry={:?} outcome={outcome} reason={reason}",
            rel.to_string_lossy()
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = (rel, outcome, reason);
}

fn trace_walk_entry(root: &Path, path: &Path, outcome: &str, reason: &str) {
    let rel = path
        .strip_prefix(root)
        .ok()
        .filter(|rel| !rel.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    trace_promote_entry(rel, outcome, reason);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(path: &Path, content: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn read(path: &Path) -> Vec<u8> {
        fs::read(path).unwrap()
    }

    fn setup() -> (TempDir, PathBuf, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let staging = tmp.path().join("staging");
        let profile = tmp.path().join("profile");
        fs::create_dir_all(&staging).unwrap();
        fs::create_dir_all(&profile).unwrap();
        (tmp, staging, profile)
    }

    fn symlink_file(source: &Path, link: &Path) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(source, link)
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(source, link)
        }
    }

    fn symlink_dir(source: &Path, link: &Path) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(source, link)
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_dir(source, link)
        }
    }

    #[test]
    fn promote_copies_allowlisted_files_and_creates_destination_subdirs() {
        let (_tmp, staging, profile) = setup();
        write(&staging.join("default/manifest.json"), b"{\"v\":1}");
        write(
            &staging.join("default/core/plugins.lock.json"),
            b"{\"plugins\":[]}",
        );
        write(
            &staging.join("default/os/macos/plugin-configs/plugin-keyremap.json"),
            b"{\"enabled\":true}",
        );
        write(
            &staging.join("default/sync/backups/20260508-conflict.json"),
            b"{\"backup\":1}",
        );
        write(&staging.join(".gitignore"), b"/active\n");

        promote_allowlisted_clone(&staging, &profile).unwrap();

        assert_eq!(read(&profile.join("default/manifest.json")), b"{\"v\":1}");
        assert_eq!(
            read(&profile.join("default/core/plugins.lock.json")),
            b"{\"plugins\":[]}"
        );
        assert_eq!(
            read(&profile.join("default/os/macos/plugin-configs/plugin-keyremap.json")),
            b"{\"enabled\":true}"
        );
        assert_eq!(
            read(&profile.join("default/sync/backups/20260508-conflict.json")),
            b"{\"backup\":1}"
        );
        assert_eq!(read(&profile.join(".gitignore")), b"/active\n");
    }

    #[test]
    fn promote_does_not_delete_local_device_subtree_or_local_sync_state_or_toggles_or_active_marker(
    ) {
        let (_tmp, staging, profile) = setup();
        write(
            &profile.join("default/device/shortcuts.json"),
            b"local-device-only",
        );
        write(&profile.join("default/device/sync/state.json"), b"\"x\"");
        write(&profile.join("default/sync/state.json"), b"\"sync-state\"");
        write(&profile.join("default/sync/toggles.json"), b"\"toggles\"");
        write(&profile.join("active"), b"default");
        write(&profile.join("sync.json"), b"{}");
        write(&profile.join("untracked-local-file.json"), b"keep me");

        write(&staging.join("default/manifest.json"), b"{\"v\":2}");

        promote_allowlisted_clone(&staging, &profile).unwrap();

        assert_eq!(
            read(&profile.join("default/device/shortcuts.json")),
            b"local-device-only",
            "device subtree must never be touched by a pull"
        );
        assert_eq!(
            read(&profile.join("default/device/sync/state.json")),
            b"\"x\"",
            "device/sync subtree must never be touched"
        );
        assert_eq!(
            read(&profile.join("default/sync/state.json")),
            b"\"sync-state\"",
            "sync/state.json is local-only and must survive a remote pull"
        );
        assert_eq!(
            read(&profile.join("default/sync/toggles.json")),
            b"\"toggles\"",
            "sync/toggles.json is local-only and must survive a remote pull"
        );
        assert_eq!(
            read(&profile.join("active")),
            b"default",
            "active marker is per-machine and must survive a remote pull"
        );
        assert_eq!(
            read(&profile.join("sync.json")),
            b"{}",
            "top-level sync.json marker must survive a remote pull"
        );
        assert_eq!(
            read(&profile.join("untracked-local-file.json")),
            b"keep me",
            "unrelated local file must never be deleted by a remote pull"
        );
        assert_eq!(
            read(&profile.join("default/manifest.json")),
            b"{\"v\":2}",
            "allowlisted file must still be promoted alongside the preservations"
        );
    }

    #[test]
    fn promote_overwrites_allowlisted_targets_when_already_present_locally() {
        let (_tmp, staging, profile) = setup();
        write(&profile.join("default/manifest.json"), b"\"old-local\"");
        write(&staging.join("default/manifest.json"), b"\"new-remote\"");

        promote_allowlisted_clone(&staging, &profile).unwrap();

        assert_eq!(
            read(&profile.join("default/manifest.json")),
            b"\"new-remote\"",
            "remote snapshot wins for allowlisted files on connect"
        );
    }

    #[test]
    fn promote_skips_unknown_top_level_files_from_staging_for_defense_in_depth() {
        let (_tmp, staging, profile) = setup();
        write(&staging.join("malicious.sh"), b"#!/bin/sh\nrm -rf $HOME");
        write(&staging.join(".bashrc"), b"alias rm='rm -rf'");
        write(&staging.join("default/manifest.json"), b"{}");

        promote_allowlisted_clone(&staging, &profile).unwrap();

        assert!(
            !profile.join("malicious.sh").exists(),
            "non-allowlisted top-level files from staging must NOT be copied"
        );
        assert!(
            !profile.join(".bashrc").exists(),
            "hidden dotfile from staging must NOT be copied"
        );
        assert!(profile.join("default/manifest.json").is_file());
    }

    #[test]
    fn promote_skips_device_subtree_from_staging_if_remote_somehow_carries_one() {
        let (_tmp, staging, profile) = setup();
        write(
            &staging.join("default/device/leaked.json"),
            b"should-not-be-applied",
        );
        write(&staging.join("default/manifest.json"), b"{\"v\":1}");

        promote_allowlisted_clone(&staging, &profile).unwrap();

        assert!(
            !profile.join("default/device/leaked.json").exists(),
            "device/ from staging is rejected even if remote somehow carries it"
        );
        assert!(profile.join("default/manifest.json").is_file());
    }

    #[test]
    fn promote_skips_sync_state_and_sync_toggles_from_staging_even_if_remote_has_them() {
        let (_tmp, staging, profile) = setup();
        write(
            &staging.join("default/sync/state.json"),
            b"\"stale-from-other-machine\"",
        );
        write(
            &staging.join("default/sync/toggles.json"),
            b"\"stale-toggles\"",
        );
        write(&profile.join("default/sync/state.json"), b"\"local\"");
        write(
            &profile.join("default/sync/toggles.json"),
            b"\"local-toggles\"",
        );

        promote_allowlisted_clone(&staging, &profile).unwrap();

        assert_eq!(
            read(&profile.join("default/sync/state.json")),
            b"\"local\"",
            "sync/state.json is per-machine; staging copy is ignored, local survives"
        );
        assert_eq!(
            read(&profile.join("default/sync/toggles.json")),
            b"\"local-toggles\"",
            "sync/toggles.json is per-machine; staging copy is ignored, local survives"
        );
    }

    #[test]
    fn promote_skips_allowlisted_symlink_file_from_staging() {
        let (_tmp, staging, profile) = setup();
        let external = staging.parent().unwrap().join("external/secret.json");
        write(&external, b"{\"source\":\"host\"}");
        write(&profile.join("default/core/plugins.lock.json"), b"local");
        fs::create_dir_all(staging.join("default/core")).unwrap();
        if symlink_file(&external, &staging.join("default/core/plugins.lock.json")).is_err() {
            return;
        }

        promote_allowlisted_clone(&staging, &profile).unwrap();

        assert_eq!(
            read(&profile.join("default/core/plugins.lock.json")),
            b"local",
            "symlinked allowlisted files from staging must not be dereferenced into the profile"
        );
    }

    #[test]
    fn promote_skips_symlink_directory_from_staging() {
        let (_tmp, staging, profile) = setup();
        let external_dir = staging.parent().unwrap().join("external-dir");
        write(&external_dir.join("leak.txt"), b"leak");
        fs::create_dir_all(staging.join("default")).unwrap();
        if symlink_dir(&external_dir, &staging.join("default/core")).is_err() {
            return;
        }
        write(&staging.join("default/manifest.json"), b"{\"v\":1}");

        promote_allowlisted_clone(&staging, &profile).unwrap();

        assert!(
            !profile.join("default/core/leak.txt").exists(),
            "symlinked directories from staging must not be traversed"
        );
        assert_eq!(read(&profile.join("default/manifest.json")), b"{\"v\":1}");
    }

    #[test]
    fn promote_git_dir_moves_clone_dotgit_into_profile_replacing_any_prior_one() {
        let (_tmp, staging, profile) = setup();
        write(&staging.join(".git/HEAD"), b"ref: refs/heads/main\n");
        write(&profile.join(".git/old"), b"should-be-replaced");

        promote_clone_git_dir(&staging, &profile).unwrap();

        assert!(!staging.join(".git").exists(), "staging .git is moved out");
        assert_eq!(
            read(&profile.join(".git/HEAD")),
            b"ref: refs/heads/main\n",
            "fresh .git replaces the old one"
        );
        assert!(
            !profile.join(".git/old").exists(),
            "old .git contents must be cleared"
        );
    }

    #[test]
    fn promote_git_dir_errors_if_staging_lacks_dotgit() {
        let (_tmp, staging, profile) = setup();
        let err = promote_clone_git_dir(&staging, &profile).unwrap_err();
        assert!(
            format!("{err:#}").contains(".git"),
            "must complain about missing .git: {err:#}"
        );
    }

    #[test]
    fn promote_errors_when_staging_dir_is_missing_so_a_failed_clone_does_not_silently_succeed() {
        let tmp = TempDir::new().unwrap();
        let staging = tmp.path().join("missing-staging");
        let profile = tmp.path().join("profile");
        fs::create_dir_all(&profile).unwrap();
        let err = promote_allowlisted_clone(&staging, &profile).unwrap_err();
        assert!(format!("{err:#}").contains("staging"));
    }
}
