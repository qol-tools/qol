use super::types::{SyncBackupEntry, SyncHealth, SyncIncident, SyncIncidentKind, SyncStatus};
use anyhow::Result;
use chrono::DateTime;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const SYNC_TOGGLES_FILE: &str = "toggles.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct SyncStateFile {
    #[serde(default)]
    pub(crate) head_sha: Option<String>,
    #[serde(default)]
    pub(crate) last_sync_at: Option<String>,
    #[serde(default)]
    pub(crate) incident: Option<SyncIncident>,
    #[serde(default)]
    pub(crate) last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct SyncToggles {
    #[serde(default = "super::types::default_true")]
    pub(crate) pull_on_launch: bool,
    #[serde(default = "super::types::default_true")]
    pub(crate) push_on_change: bool,
}

impl Default for SyncToggles {
    fn default() -> Self {
        Self {
            pull_on_launch: true,
            push_on_change: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PullMode {
    Launch,
    Manual,
}

impl PullMode {
    pub(crate) fn incident_kind(self) -> SyncIncidentKind {
        match self {
            Self::Launch => SyncIncidentKind::LaunchPullReview,
            Self::Manual => SyncIncidentKind::ManualPullReview,
        }
    }
}

pub(crate) fn build_status(
    state: &SyncStateFile,
    target: Option<&crate::features::profile::registry::SyncTarget>,
    toggles: SyncToggles,
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
        has_github_token: crate::credentials::github_bearer_token().is_some(),
        head_sha: state.head_sha.clone(),
        last_sync_at: state.last_sync_at.clone(),
        incident: state.incident.clone(),
        last_error: state.last_error.clone(),
        backups_dir: backups_dir.map(|path| path.display().to_string()),
        backup_count: backup_files.len(),
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

pub(crate) fn ensure_sync_dirs() -> Result<()> {
    std::fs::create_dir_all(crate::paths::sync_dir()?)?;
    std::fs::create_dir_all(crate::paths::sync_backups_dir()?)?;
    Ok(())
}

pub(crate) fn load_state_file() -> Result<SyncStateFile> {
    let path = crate::paths::sync_state_path()?;
    if !path.exists() {
        return Ok(SyncStateFile::default());
    }
    crate::file_io::read_json(&path)
}

pub(crate) fn save_state_file(state: &SyncStateFile) -> Result<()> {
    ensure_sync_dirs()?;
    crate::file_io::write_pretty_json(&crate::paths::sync_state_path()?, state)
}

pub(crate) fn toggles_path() -> Result<PathBuf> {
    crate::paths::sync_dir().map(|p| p.join(SYNC_TOGGLES_FILE))
}

pub(crate) fn load_toggles() -> Result<SyncToggles> {
    let path = toggles_path()?;
    if !path.exists() {
        return Ok(SyncToggles::default());
    }
    crate::file_io::read_json(&path)
}

pub(crate) fn save_toggles(toggles: SyncToggles) -> Result<()> {
    ensure_sync_dirs()?;
    crate::file_io::write_pretty_json(&toggles_path()?, &toggles)
}

pub(crate) fn list_backup_entries(dir: Option<&Path>) -> Vec<SyncBackupEntry> {
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

pub(crate) fn filename_string(path: &Path) -> String {
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

pub(crate) fn backup_file_path(file_name: &str) -> Result<PathBuf> {
    let safe = file_name.trim();
    if safe.is_empty() || safe.contains('/') || safe.contains('\\') {
        anyhow::bail!("invalid backup file name");
    }
    Ok(crate::paths::sync_backups_dir()?.join(safe))
}

pub(crate) fn now_rfc3339() -> String {
    let datetime: DateTime<chrono::Utc> = std::time::SystemTime::now().into();
    datetime.to_rfc3339()
}
