use super::scope::{current_os_bucket, OS_SUBDIR};
use super::types::{
    default_true, ResolvableConflict, SyncBackupEntry, SyncHealth, SyncIncident, SyncStatus,
    SyncTarget,
};
use anyhow::{Context, Result};
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

const DEFAULT_PROFILE_NAME: &str = "default";
const ACTIVE_PROFILE_FILE: &str = "active";
const SYNC_STATE_FILE: &str = "state.json";
const SYNC_TOGGLES_FILE: &str = "toggles.json";
const SYNC_CONFIG_FILE: &str = "sync.json";
const LOCK_FILE: &str = "sync.lock";
const HOTKEYS_FILE: &str = "hotkeys.json";

/// Resolved paths for one machine's sync state, derived from the profile
/// root (the git repository that holds the `default/`-style profile
/// directories). Consumers resolve the config directory themselves and hand
/// the profile root in as plain configuration.
#[derive(Debug, Clone)]
pub struct SyncPaths {
    profile_root: PathBuf,
}

impl SyncPaths {
    pub fn new(profile_root: PathBuf) -> Self {
        Self { profile_root }
    }

    pub fn profile_root(&self) -> &Path {
        &self.profile_root
    }

    /// Active profile name from the `<profile>/active` marker, with the same
    /// safe-component validation the tray applies everywhere else.
    pub fn active_profile_name(&self) -> String {
        let marker = self.profile_root.join(ACTIVE_PROFILE_FILE);
        std::fs::read_to_string(marker)
            .ok()
            .map(|raw| raw.trim().to_string())
            .filter(|name| qol_plugin_api::manifest::is_valid_safe_identifier(name))
            .unwrap_or_else(|| DEFAULT_PROFILE_NAME.to_string())
    }

    pub fn active_dir(&self) -> PathBuf {
        self.profile_root.join(self.active_profile_name())
    }

    /// Per-device sync state dir: `<profile>/<name>/device/sync`.
    pub fn sync_dir(&self) -> PathBuf {
        self.active_dir().join("device").join("sync")
    }

    pub fn os_dir(&self) -> PathBuf {
        self.active_dir().join(OS_SUBDIR).join(current_os_bucket())
    }

    pub fn hotkeys_path(&self) -> PathBuf {
        self.os_dir().join(HOTKEYS_FILE)
    }

    pub fn state_path(&self) -> PathBuf {
        self.sync_dir().join(SYNC_STATE_FILE)
    }

    pub fn toggles_path(&self) -> PathBuf {
        self.sync_dir().join(SYNC_TOGGLES_FILE)
    }

    /// Tracked conflict-backup dir: `<profile>/<name>/sync/backups`.
    pub fn backups_dir(&self) -> PathBuf {
        self.active_dir().join("sync").join("backups")
    }

    /// Cross-process lockfile inside the per-device sync state dir. See the
    /// crate docs for the lock contract.
    pub fn lock_path(&self) -> PathBuf {
        self.sync_dir().join(LOCK_FILE)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncStateFile {
    #[serde(default)]
    pub head_sha: Option<String>,
    #[serde(default)]
    pub last_sync_at: Option<String>,
    #[serde(default)]
    pub incident: Option<SyncIncident>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub conflicts: Vec<ResolvableConflict>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SyncToggles {
    #[serde(default = "default_true")]
    pub pull_on_launch: bool,
    #[serde(default = "default_true")]
    pub push_on_change: bool,
}

impl Default for SyncToggles {
    fn default() -> Self {
        Self {
            pull_on_launch: true,
            push_on_change: true,
        }
    }
}

pub fn build_status(
    state: &SyncStateFile,
    target: Option<&SyncTarget>,
    toggles: SyncToggles,
    has_github_token: bool,
    backups_dir: Option<&Path>,
    backup_files: &[SyncBackupEntry],
) -> SyncStatus {
    let configured = target.is_some();
    let health = derive_health(configured, state);
    SyncStatus {
        configured,
        repo_url: target.map(|t| t.repo_url.clone()),
        auto_created: target.map(|t| t.auto_created).unwrap_or(false),
        health,
        pull_on_launch: toggles.pull_on_launch,
        push_on_change: toggles.push_on_change,
        has_github_token,
        head_sha: state.head_sha.clone(),
        last_sync_at: state.last_sync_at.clone(),
        incident: state.incident.clone(),
        last_error: state.last_error.clone(),
        backups_dir: backups_dir.map(|path| path.display().to_string()),
        backup_count: backup_files.len(),
        conflict_count: state.conflicts.len(),
        latest_backup_file: backup_files.first().map(|entry| entry.file_name.clone()),
    }
}

fn derive_health(configured: bool, state: &SyncStateFile) -> SyncHealth {
    if !configured {
        return SyncHealth::NotConfigured;
    }
    if state.last_error.is_some() {
        return SyncHealth::Error;
    }
    if state.incident.is_some() {
        return SyncHealth::Attention;
    }
    SyncHealth::Healthy
}

pub fn ensure_sync_dirs(paths: &SyncPaths) -> Result<()> {
    std::fs::create_dir_all(paths.sync_dir())?;
    std::fs::create_dir_all(paths.backups_dir())?;
    Ok(())
}

pub fn load_state_file(paths: &SyncPaths) -> Result<SyncStateFile> {
    let path = paths.state_path();
    if !path.exists() {
        return Ok(SyncStateFile::default());
    }
    read_json(&path)
}

pub fn save_state_file(paths: &SyncPaths, state: &SyncStateFile) -> Result<()> {
    ensure_sync_dirs(paths)?;
    write_pretty_json(&paths.state_path(), state)
}

pub fn load_toggles(paths: &SyncPaths) -> Result<SyncToggles> {
    let path = paths.toggles_path();
    if !path.exists() {
        return Ok(SyncToggles::default());
    }
    read_json(&path)
}

pub fn save_toggles(paths: &SyncPaths, toggles: SyncToggles) -> Result<()> {
    ensure_sync_dirs(paths)?;
    write_pretty_json(&paths.toggles_path(), &toggles)
}

pub fn load_sync_target(profile_root: &Path) -> Result<Option<SyncTarget>> {
    let path = profile_root.join(SYNC_CONFIG_FILE);
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

pub fn save_sync_target(profile_root: &Path, target: &SyncTarget) -> Result<()> {
    write_pretty_json(&profile_root.join(SYNC_CONFIG_FILE), target)
}

pub fn clear_sync_target(profile_root: &Path) -> Result<()> {
    let path = profile_root.join(SYNC_CONFIG_FILE);
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

pub fn list_backup_entries(dir: Option<&Path>) -> Vec<SyncBackupEntry> {
    let Some(dir) = dir else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<SyncBackupEntry> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            let file_name = filename_string(&path);
            let metadata = entry.metadata().ok()?;
            Some(SyncBackupEntry {
                created_at: backup_timestamp(&file_name),
                file_name,
                size_bytes: metadata.len(),
            })
        })
        .collect();
    files.sort_by(|a, b| b.file_name.cmp(&a.file_name));
    files
}

pub fn filename_string(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_string()
}

fn backup_timestamp(file_name: &str) -> String {
    file_name
        .split_once('-')
        .map(|(date, _)| date.to_string())
        .unwrap_or_default()
}

pub fn backup_file_path(paths: &SyncPaths, file_name: &str) -> Result<PathBuf> {
    let safe = file_name.trim();
    if let Some(reason) = backup_file_reject_reason(safe) {
        trace_backup_path(safe, "reject", Some(reason));
        anyhow::bail!("invalid backup file name");
    }
    trace_backup_path(safe, "accept", None);
    Ok(paths.backups_dir().join(safe))
}

fn backup_file_reject_reason(file_name: &str) -> Option<&'static str> {
    if file_name.is_empty() {
        Some("empty")
    } else if file_name == "." || file_name == ".." {
        Some("dot_entry")
    } else if file_name.contains('/') || file_name.contains('\\') {
        Some("separator")
    } else {
        None
    }
}

/// Writes a local+remote snapshot backup into the tracked backups dir and
/// returns its file name. The `<timestamp>-conflict.json` naming is part of
/// the shared contract every sync consumer relies on.
pub fn write_conflict_backup(paths: &SyncPaths, value: &Value) -> Result<String> {
    ensure_sync_dirs(paths)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let name = format!("{stamp}-conflict.json");
    write_pretty_json(&paths.backups_dir().join(&name), value)?;
    Ok(name)
}

pub fn now_rfc3339() -> String {
    let datetime: DateTime<chrono::Utc> = std::time::SystemTime::now().into();
    datetime.to_rfc3339()
}

fn trace_backup_path(file_name: &str, outcome: &str, reason: Option<&str>) {
    #[cfg(debug_assertions)]
    {
        if let Some(reason) = reason {
            qol_runtime::probe!(
                "PROFILE_BACKUP_PATH",
                "file={:?} outcome={outcome} reason={reason}",
                file_name
            );
        } else {
            qol_runtime::probe!(
                "PROFILE_BACKUP_PATH",
                "file={:?} outcome={outcome}",
                file_name
            );
        }
    }
    #[cfg(not(debug_assertions))]
    let _ = (file_name, outcome, reason);
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let content = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

fn write_pretty_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dir {}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(value)?;
    qol_fs::atomic_write(path, content.as_bytes())
        .with_context(|| format!("write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn paths_in(tmp: &TempDir) -> SyncPaths {
        SyncPaths::new(tmp.path().join("profile"))
    }

    #[test]
    fn sync_paths_compose_the_profile_layout() {
        let tmp = TempDir::new().unwrap();
        let paths = paths_in(&tmp);
        std::fs::create_dir_all(paths.active_dir().join("work")).unwrap();
        std::fs::write(paths.profile_root.join("active"), "work\n").unwrap();

        assert!(paths.active_profile_name() == "work");
        assert!(paths.sync_dir().ends_with("work/device/sync"));
        assert!(paths.state_path().ends_with("work/device/sync/state.json"));
        assert!(paths.backups_dir().ends_with("work/sync/backups"));
        assert!(paths.lock_path().ends_with("work/device/sync/sync.lock"));
    }

    #[test]
    fn os_dir_and_hotkeys_path_are_os_scoped_in_the_active_profile() {
        let tmp = TempDir::new().unwrap();
        let paths = paths_in(&tmp);
        std::fs::create_dir_all(paths.active_dir()).unwrap();
        std::fs::write(paths.profile_root.join("active"), "work\n").unwrap();

        let bucket = current_os_bucket();
        assert!(paths.os_dir().ends_with(format!("work/os/{bucket}")));
        assert!(paths
            .hotkeys_path()
            .ends_with(format!("work/os/{bucket}/hotkeys.json")));
    }

    #[test]
    fn active_profile_name_falls_back_on_invalid_marker_content() {
        let tmp = TempDir::new().unwrap();
        let paths = paths_in(&tmp);
        std::fs::create_dir_all(paths.profile_root()).unwrap();
        std::fs::write(paths.profile_root.join("active"), "../escape\n").unwrap();
        assert_eq!(paths.active_profile_name(), DEFAULT_PROFILE_NAME);
    }

    #[test]
    fn backup_file_path_accepts_plain_backup_file_names() {
        let tmp = TempDir::new().unwrap();
        let paths = paths_in(&tmp);

        let path = backup_file_path(&paths, "20260508-conflict.json").unwrap();

        assert!(path.ends_with("sync/backups/20260508-conflict.json"));
    }

    #[test]
    fn backup_file_path_rejects_traversal_and_path_components() {
        let tmp = TempDir::new().unwrap();
        let paths = paths_in(&tmp);
        let invalid_names = [
            "",
            " ",
            ".",
            " . ",
            "..",
            " .. ",
            "../backup.json",
            "subdir/backup.json",
            "subdir\\backup.json",
            "/absolute.json",
            "\\absolute.json",
        ];

        for file_name in invalid_names {
            assert!(
                backup_file_path(&paths, file_name).is_err(),
                "backup file name should be rejected: {file_name:?}"
            );
        }
    }

    #[test]
    fn state_file_roundtrips_through_the_device_sync_dir() {
        let tmp = TempDir::new().unwrap();
        let paths = paths_in(&tmp);
        let state = SyncStateFile {
            head_sha: Some("abc123".to_string()),
            last_sync_at: Some("2026-01-01T00:00:00Z".to_string()),
            incident: None,
            last_error: None,
            conflicts: vec![ResolvableConflict {
                file: "default/core/plugin-configs/a.json".to_string(),
                plugin: Some("plugin-a".to_string()),
                key_path: "value".to_string(),
                local: serde_json::json!(1),
                remote: serde_json::json!(2),
                local_edited: None,
                remote_edited: None,
            }],
        };
        save_state_file(&paths, &state).unwrap();
        assert_eq!(load_state_file(&paths).unwrap(), state);
    }

    #[test]
    fn write_conflict_backup_lands_in_backups_dir_and_lists_first() {
        let tmp = TempDir::new().unwrap();
        let paths = paths_in(&tmp);

        let name = write_conflict_backup(&paths, &serde_json::json!({"local": 1})).unwrap();
        assert!(name.ends_with("-conflict.json"));

        let entries = list_backup_entries(Some(&paths.backups_dir()));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].file_name, name);
        assert!(entries[0].size_bytes > 0);
    }

    #[test]
    fn status_reports_attention_while_an_incident_is_pending() {
        let tmp = TempDir::new().unwrap();
        let paths = paths_in(&tmp);
        let target = SyncTarget {
            repo_url: "https://github.com/me/qol-tray-profiles".to_string(),
            auto_created: false,
        };
        let state = SyncStateFile {
            incident: Some(SyncIncident {
                kind: super::super::types::SyncIncidentKind::Conflict,
                message: "1 setting(s) differ".to_string(),
                backup_file: None,
                created_at: now_rfc3339(),
            }),
            ..SyncStateFile::default()
        };
        let backups_dir = paths.backups_dir();
        let status = build_status(
            &state,
            Some(&target),
            SyncToggles::default(),
            true,
            Some(&backups_dir),
            &list_backup_entries(Some(&backups_dir)),
        );
        assert_eq!(status.health, SyncHealth::Attention);
        assert_eq!(status.conflict_count, 0);
        assert_eq!(
            status.repo_url.as_deref(),
            Some("https://github.com/me/qol-tray-profiles")
        );
        assert!(status.has_github_token);
    }
}
