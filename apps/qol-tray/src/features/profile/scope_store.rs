use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::features::profile::core::PluginLockEntry;
use crate::paths::{self, is_safe_path_component};
use crate::plugins::config::{classify_os_bucket, resolve_plugin_config, PluginConfigResolution};
use crate::plugins::manifest::{ConfigScope, PluginManifest};

pub(crate) const CORE_SUBDIR: &str = "core";
pub(crate) const OS_SUBDIR: &str = "os";
pub(crate) const DEVICE_SUBDIR: &str = "device";
pub(crate) const SYNC_SUBDIR: &str = "sync";
pub(crate) const BACKUPS_SUBDIR: &str = "backups";
pub(crate) const PLUGIN_CONFIGS_SUBDIR: &str = "plugin-configs";

pub(crate) const MANIFEST_FILE: &str = "manifest.json";
pub(crate) const PLUGINS_LOCK_FILE: &str = "plugins.lock.json";
pub(crate) const HOTKEYS_FILE: &str = "hotkeys.json";
pub(crate) const SHORTCUTS_FILE: &str = "shortcuts.json";
pub(crate) const TASK_RUNNER_FILE: &str = "task-runner.json";
pub(crate) const SYNC_STATE_FILE: &str = "state.json";
pub(crate) const SYNC_TOGGLES_FILE: &str = "toggles.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScopeKind {
    Core,
    Os,
    Device,
}

impl ScopeKind {
    pub fn subdir_name(self) -> &'static str {
        match self {
            Self::Core => CORE_SUBDIR,
            Self::Os => OS_SUBDIR,
            Self::Device => DEVICE_SUBDIR,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProfileScopeStore {
    profile_root: PathBuf,
    profile_name: String,
    os_bucket: String,
}

impl ProfileScopeStore {
    pub fn from_active() -> Result<Self> {
        Self::new(
            paths::profile_dir()?,
            paths::active_profile_name(),
            paths::current_os_subdir().to_string(),
        )
    }

    pub fn new(profile_root: PathBuf, profile_name: String, os_bucket: String) -> Result<Self> {
        if !is_safe_path_component(&profile_name) {
            return Err(anyhow!("invalid profile name: {profile_name}"));
        }
        if !is_safe_path_component(&os_bucket) {
            return Err(anyhow!("invalid os bucket: {os_bucket}"));
        }
        Ok(Self {
            profile_root,
            profile_name,
            os_bucket,
        })
    }

    pub fn at_dir(profile_dir: PathBuf, os_bucket: String) -> Result<Self> {
        if !is_safe_path_component(&os_bucket) {
            return Err(anyhow!("invalid os bucket: {os_bucket}"));
        }
        let profile_name = profile_dir
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| {
                anyhow!(
                    "profile_dir has no usable name component: {}",
                    profile_dir.display()
                )
            })?
            .to_string();
        let profile_root = profile_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Ok(Self {
            profile_root,
            profile_name,
            os_bucket,
        })
    }

    pub fn profile_root(&self) -> &Path {
        &self.profile_root
    }

    pub fn profile_name(&self) -> &str {
        &self.profile_name
    }

    pub fn os_bucket(&self) -> &str {
        &self.os_bucket
    }

    pub fn dir(&self) -> PathBuf {
        self.profile_root.join(&self.profile_name)
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.dir().join(MANIFEST_FILE)
    }

    pub fn scope_dir(&self, scope: ScopeKind) -> PathBuf {
        match scope {
            ScopeKind::Core => self.core_dir(),
            ScopeKind::Os => self.os_dir(),
            ScopeKind::Device => self.device_dir(),
        }
    }

    pub fn core_dir(&self) -> PathBuf {
        self.dir().join(CORE_SUBDIR)
    }

    pub fn os_dir(&self) -> PathBuf {
        self.os_dir_for(&self.os_bucket)
    }

    pub fn os_dir_for(&self, bucket: &str) -> PathBuf {
        self.dir().join(OS_SUBDIR).join(bucket)
    }

    pub fn device_dir(&self) -> PathBuf {
        self.dir().join(DEVICE_SUBDIR)
    }

    pub fn plugins_lock_path(&self) -> PathBuf {
        self.core_dir().join(PLUGINS_LOCK_FILE)
    }

    pub fn core_plugin_configs_dir(&self) -> PathBuf {
        self.core_dir().join(PLUGIN_CONFIGS_SUBDIR)
    }

    pub fn core_plugin_config_path(&self, plugin_id: &str) -> Result<PathBuf> {
        validate_plugin_id(plugin_id)?;
        Ok(self
            .core_plugin_configs_dir()
            .join(format!("{}.json", plugin_id)))
    }

    pub fn device_plugin_configs_dir(&self) -> PathBuf {
        self.device_dir().join(PLUGIN_CONFIGS_SUBDIR)
    }

    pub fn device_plugin_config_path(&self, plugin_id: &str) -> Result<PathBuf> {
        validate_plugin_id(plugin_id)?;
        Ok(self
            .device_plugin_configs_dir()
            .join(format!("{}.json", plugin_id)))
    }

    pub fn os_plugin_configs_dir(&self) -> PathBuf {
        self.os_plugin_configs_dir_for(&self.os_bucket)
    }

    pub fn os_plugin_configs_dir_for(&self, bucket: &str) -> PathBuf {
        self.os_dir_for(bucket).join(PLUGIN_CONFIGS_SUBDIR)
    }

    pub fn os_plugin_config_path(&self, plugin_id: &str) -> Result<PathBuf> {
        self.os_plugin_config_path_for(plugin_id, &self.os_bucket)
    }

    pub fn os_plugin_config_path_for(&self, plugin_id: &str, bucket: &str) -> Result<PathBuf> {
        validate_plugin_id(plugin_id)?;
        if !is_safe_path_component(bucket) {
            return Err(anyhow!("invalid os_bucket: {bucket}"));
        }
        Ok(self
            .os_plugin_configs_dir_for(bucket)
            .join(format!("{}.json", plugin_id)))
    }

    pub fn hotkeys_path(&self) -> PathBuf {
        self.os_dir().join(HOTKEYS_FILE)
    }

    pub fn shortcuts_path(&self) -> PathBuf {
        self.os_dir().join(SHORTCUTS_FILE)
    }

    pub fn task_runner_path(&self) -> PathBuf {
        self.os_dir().join(TASK_RUNNER_FILE)
    }

    pub fn device_sync_dir(&self) -> PathBuf {
        self.device_dir().join(SYNC_SUBDIR)
    }

    pub fn sync_state_path(&self) -> PathBuf {
        self.device_sync_dir().join(SYNC_STATE_FILE)
    }

    pub fn sync_toggles_path(&self) -> PathBuf {
        self.device_sync_dir().join(SYNC_TOGGLES_FILE)
    }

    pub fn sync_backups_dir(&self) -> PathBuf {
        self.dir().join(SYNC_SUBDIR).join(BACKUPS_SUBDIR)
    }

    pub fn plugin_config_resolution(
        &self,
        lock_entry: Option<&PluginLockEntry>,
        manifest: Option<&PluginManifest>,
    ) -> Result<PluginConfigResolution> {
        resolve_plugin_config(lock_entry, manifest, &self.os_bucket)
    }

    pub fn plugin_config_target_path(
        &self,
        plugin_id: &str,
        lock_entry: Option<&PluginLockEntry>,
        manifest: Option<&PluginManifest>,
    ) -> Result<PathBuf> {
        let resolution = self.plugin_config_resolution(lock_entry, manifest)?;
        match resolution.scope {
            ConfigScope::Core => self.core_plugin_config_path(plugin_id),
            ConfigScope::Device => self.device_plugin_config_path(plugin_id),
            ConfigScope::Os => {
                let bucket = resolution
                    .os_bucket
                    .ok_or_else(|| anyhow!("internal: Os scope resolved without an os_bucket"))?;
                self.os_plugin_config_path_for(plugin_id, &bucket)
            }
        }
    }

    pub fn plugin_config_slice_paths(
        &self,
        plugin_id: &str,
        lock_entry: Option<&PluginLockEntry>,
        manifest: Option<&PluginManifest>,
    ) -> Result<PluginConfigSlicePaths> {
        let os_bucket = classify_os_bucket(lock_entry, manifest, &self.os_bucket)?;
        Ok(PluginConfigSlicePaths {
            core: self.core_plugin_config_path(plugin_id)?,
            os: self.os_plugin_config_path_for(plugin_id, &os_bucket)?,
            device: self.device_plugin_config_path(plugin_id)?,
            os_bucket,
        })
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        let dirs = [
            self.dir(),
            self.core_dir(),
            self.core_plugin_configs_dir(),
            self.os_dir(),
            self.device_dir(),
        ];
        for dir in dirs {
            std::fs::create_dir_all(&dir)
                .map_err(|e| anyhow!("create profile subdir {}: {e}", dir.display()))?;
        }
        Ok(())
    }

    pub fn is_sync_allowlisted(rel: &Path) -> bool {
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginConfigSlicePaths {
    pub core: PathBuf,
    pub os: PathBuf,
    pub device: PathBuf,
    pub os_bucket: String,
}

fn validate_plugin_id(plugin_id: &str) -> Result<()> {
    if !is_safe_path_component(plugin_id) {
        return Err(anyhow!("invalid plugin id: {plugin_id}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store(name: &str, os: &str) -> (TempDir, ProfileScopeStore) {
        let tmp = TempDir::new().unwrap();
        let store =
            ProfileScopeStore::new(tmp.path().to_path_buf(), name.to_string(), os.to_string())
                .unwrap();
        (tmp, store)
    }

    #[test]
    fn dir_lookups_compose_root_name_and_scope() {
        let (_tmp, s) = store("work", "macos");
        let root = s.profile_root().to_path_buf();
        let active = root.join("work");

        assert_eq!(s.dir(), active);
        assert_eq!(s.manifest_path(), active.join("manifest.json"));
        assert_eq!(s.core_dir(), active.join("core"));
        assert_eq!(s.os_dir(), active.join("os/macos"));
        assert_eq!(s.os_dir_for("linux"), active.join("os/linux"));
        assert_eq!(s.device_dir(), active.join("device"));
        assert_eq!(s.plugins_lock_path(), active.join("core/plugins.lock.json"));
        assert_eq!(
            s.core_plugin_configs_dir(),
            active.join("core/plugin-configs")
        );
        assert_eq!(s.hotkeys_path(), active.join("os/macos/hotkeys.json"));
        assert_eq!(s.shortcuts_path(), active.join("os/macos/shortcuts.json"));
        assert_eq!(
            s.task_runner_path(),
            active.join("os/macos/task-runner.json")
        );
        assert_eq!(s.device_sync_dir(), active.join("device/sync"));
        assert_eq!(s.sync_state_path(), active.join("device/sync/state.json"));
        assert_eq!(
            s.sync_toggles_path(),
            active.join("device/sync/toggles.json")
        );
        assert_eq!(s.sync_backups_dir(), active.join("sync/backups"));
    }

    #[test]
    fn scope_dir_dispatch_returns_same_path_as_named_getter() {
        let (_tmp, s) = store("default", "linux");
        assert_eq!(s.scope_dir(ScopeKind::Core), s.core_dir());
        assert_eq!(s.scope_dir(ScopeKind::Os), s.os_dir());
        assert_eq!(s.scope_dir(ScopeKind::Device), s.device_dir());
    }

    #[test]
    fn core_plugin_config_path_rejects_unsafe_id() {
        let (_tmp, s) = store("default", "linux");
        for evil in ["../etc/passwd", "with/slash", "", ".hidden", "-leading"] {
            let err = s.core_plugin_config_path(evil).unwrap_err();
            assert!(
                format!("{err:#}").contains("invalid plugin id"),
                "should reject {evil:?}: {err:#}"
            );
        }
    }

    #[test]
    fn new_rejects_unsafe_profile_name_and_os_bucket() {
        let tmp = TempDir::new().unwrap();
        let bad = ["", "../escape", "with space", "-leading", "with/slash"];
        for name in bad {
            assert!(
                ProfileScopeStore::new(
                    tmp.path().to_path_buf(),
                    name.to_string(),
                    "linux".to_string()
                )
                .is_err(),
                "should reject profile name {name:?}"
            );
            assert!(
                ProfileScopeStore::new(
                    tmp.path().to_path_buf(),
                    "default".to_string(),
                    name.to_string()
                )
                .is_err(),
                "should reject os bucket {name:?}"
            );
        }
    }

    #[test]
    fn ensure_dirs_creates_the_full_layout_idempotently() {
        let (_tmp, s) = store("work", "macos");
        for _ in 0..2 {
            s.ensure_dirs().unwrap();
        }
        for path in [
            s.dir(),
            s.core_dir(),
            s.core_plugin_configs_dir(),
            s.os_dir(),
            s.device_dir(),
        ] {
            assert!(path.is_dir(), "{} should exist", path.display());
        }
    }

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
            assert!(
                ProfileScopeStore::is_sync_allowlisted(Path::new(raw)),
                "must allowlist: {raw}"
            );
        }
    }

    #[test]
    fn allowlist_rejects_device_subtree_sync_state_toggles_active_marker_and_unknowns() {
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
        ];
        for raw in cases {
            assert!(
                !ProfileScopeStore::is_sync_allowlisted(Path::new(raw)),
                "must not allowlist: {raw}"
            );
        }
    }

    #[test]
    fn plugin_config_slice_paths_use_store_os_bucket_when_no_signals_provided() {
        let (_tmp, s) = store("default", "macos");
        let paths = s.plugin_config_slice_paths("plugin-x", None, None).unwrap();
        let active = s.dir();
        assert_eq!(paths.os_bucket, "macos");
        assert_eq!(paths.core, active.join("core/plugin-configs/plugin-x.json"));
        assert_eq!(
            paths.os,
            active.join("os/macos/plugin-configs/plugin-x.json")
        );
        assert_eq!(
            paths.device,
            active.join("device/plugin-configs/plugin-x.json")
        );
    }

    #[test]
    fn plugin_config_slice_paths_use_lock_single_platform_even_from_another_os() {
        let (_tmp, s) = store("default", "linux");
        let lock = PluginLockEntry {
            id: "plugin-keyremap".to_string(),
            repo_url: "https://example/r".to_string(),
            version: "1.0.0".to_string(),
            platforms: Some(vec!["macos".to_string()]),
        };
        let paths = s
            .plugin_config_slice_paths("plugin-keyremap", Some(&lock), None)
            .unwrap();
        let active = s.dir();
        assert_eq!(paths.os_bucket, "macos");
        assert_eq!(
            paths.os,
            active.join("os/macos/plugin-configs/plugin-keyremap.json"),
            "Linux machine must materialize Mac-only plugin's os slice under os/macos"
        );
    }

    #[test]
    fn plugin_config_target_path_dispatches_on_resolution_scope() {
        let (_tmp, s) = store("default", "linux");
        let target = s.plugin_config_target_path("plugin-x", None, None).unwrap();
        assert_eq!(
            target,
            s.dir().join("core/plugin-configs/plugin-x.json"),
            "no signals -> Core scope target"
        );

        let mac_only_lock = PluginLockEntry {
            id: "plugin-keyremap".to_string(),
            repo_url: "https://example/r".to_string(),
            version: "1.0.0".to_string(),
            platforms: Some(vec!["macos".to_string()]),
        };
        let target = s
            .plugin_config_target_path("plugin-keyremap", Some(&mac_only_lock), None)
            .unwrap();
        assert_eq!(
            target,
            s.dir().join("os/macos/plugin-configs/plugin-keyremap.json"),
            "single-platform lock -> Os scope target on the declared bucket"
        );
    }

    #[test]
    fn at_dir_recovers_name_and_root_from_combined_profile_dir() {
        let tmp = TempDir::new().unwrap();
        let profile_dir = tmp.path().join("work");
        let s = ProfileScopeStore::at_dir(profile_dir.clone(), "linux".to_string()).unwrap();
        assert_eq!(s.profile_name(), "work");
        assert_eq!(s.os_bucket(), "linux");
        assert_eq!(s.dir(), profile_dir);
        assert_eq!(s.core_dir(), profile_dir.join("core"));
    }

    #[test]
    fn at_dir_still_validates_os_bucket_even_when_skipping_name_validation() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("work");
        for evil in ["../etc", "with/slash", "", ".hidden"] {
            assert!(
                ProfileScopeStore::at_dir(dir.clone(), evil.to_string()).is_err(),
                "must reject os_bucket {evil:?}"
            );
        }
    }
}
