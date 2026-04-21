use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use super::types::{SyncBackupEntry, SyncHealth, SyncIncident, SyncStatus};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct SyncStateFile {
    #[serde(default)]
    pub(crate) connection: Option<super::types::SyncConnection>,
    #[serde(default)]
    pub(crate) remote_revision: Option<String>,
    #[serde(default)]
    pub(crate) last_synced_hash: Option<String>,
    #[serde(default)]
    pub(crate) last_sync_at: Option<String>,
    #[serde(default)]
    pub(crate) incident: Option<SyncIncident>,
    #[serde(default)]
    pub(crate) last_error: Option<String>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum PullMode {
    Launch,
    Manual,
    Connect,
}

pub(crate) fn build_status(state: &SyncStateFile) -> SyncStatus {
    let backups_dir = crate::paths::sync_backups_dir().ok();
    let backup_files = list_backup_entries(backups_dir.as_deref());
    let connection = state.connection.as_ref();
    let health = if connection.is_none() {
        SyncHealth::NotConfigured
    } else if state.last_error.is_some() {
        SyncHealth::Error
    } else if state.incident.is_some() {
        SyncHealth::Attention
    } else {
        SyncHealth::Healthy
    };

    SyncStatus {
        configured: connection.is_some(),
        provider: connection.map(super::types::SyncConnection::provider_kind),
        provider_label: connection.map(|connection| connection.provider_label().to_string()),
        target_summary: connection
            .map(super::types::SyncConnection::target_summary)
            .filter(|value| !value.is_empty()),
        health,
        gist_id: connection
            .and_then(super::types::SyncConnection::gist_id)
            .map(str::to_string)
            .filter(|value| !value.is_empty()),
        folder_path: connection
            .and_then(super::types::SyncConnection::folder_path)
            .map(str::to_string)
            .filter(|value| !value.is_empty()),
        path: connection
            .and_then(super::types::SyncConnection::path)
            .map(str::to_string)
            .filter(|value| !value.is_empty()),
        pull_on_launch: connection
            .map(super::types::SyncConnection::pull_on_launch)
            .unwrap_or(false),
        push_on_change: connection
            .map(super::types::SyncConnection::push_on_change)
            .unwrap_or(false),
        has_github_token: crate::credentials::github_bearer_token().is_some(),
        last_sync_at: state.last_sync_at.clone(),
        incident: state.incident.clone(),
        last_error: state.last_error.clone(),
        backups_dir: backups_dir.map(|path| path.display().to_string()),
        backup_count: backup_files.len(),
        latest_backup_file: backup_files.first().map(|entry| entry.file_name.clone()),
    }
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

pub(crate) fn list_backup_entries(dir: Option<&Path>) -> Vec<SyncBackupEntry> {
    let Some(dir) = dir else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };

    let mut files = entries
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
        .collect::<Vec<_>>();
    files.sort_by(|left, right| right.file_name.cmp(&left.file_name));
    files
}

pub(crate) fn filename_string(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

fn backup_timestamp(file_name: &str) -> String {
    let Some(stem) = file_name.strip_suffix(".json") else {
        return file_name.to_string();
    };
    let Some((date, rest)) = stem.split_once('-') else {
        return file_name.to_string();
    };
    let timestamp = format!("{date}-{rest}");
    let prefix = &timestamp[..timestamp.len().min(15)];
    let Ok(value) = chrono::NaiveDateTime::parse_from_str(prefix, "%Y%m%d-%H%M%S") else {
        return file_name.to_string();
    };
    value.format("%Y-%m-%d %H:%M:%S").to_string()
}

pub(crate) fn backup_file_path(file_name: &str) -> Result<PathBuf> {
    validate_backup_file_name(file_name)?;
    Ok(crate::paths::sync_backups_dir()?.join(file_name))
}

fn validate_backup_file_name(file_name: &str) -> Result<()> {
    let trimmed = file_name.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Backup file name is required");
    }
    if !trimmed.ends_with(".json") {
        anyhow::bail!("Invalid backup file name");
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains("..") {
        anyhow::bail!("Invalid backup file name");
    }
    let valid = trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'));
    if !valid {
        anyhow::bail!("Invalid backup file name");
    }
    Ok(())
}

pub(crate) fn sanitize_reason(reason: &str) -> String {
    let normalized = reason
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                return ch;
            }
            '-'
        })
        .collect::<String>();
    if normalized.is_empty() {
        return "backup".to_string();
    }
    normalized
}

pub(crate) fn hash_text(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(crate) fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339()
}

pub(crate) fn pull_noop_message(mode: PullMode) -> String {
    match mode {
        PullMode::Launch => "Cloud sync checked remote profile on launch".to_string(),
        PullMode::Manual => "Cloud sync is already up to date".to_string(),
        PullMode::Connect => "Cloud sync connected to an already-synced profile".to_string(),
    }
}

pub(crate) fn pull_success_message(
    mode: PullMode,
    local_dirty: bool,
    apply_success: bool,
) -> String {
    if matches!(mode, PullMode::Connect) && local_dirty {
        if apply_success {
            return "Remote profile was applied during setup. Local profile was backed up for review."
                .to_string();
        }
        return "Remote profile was applied during setup with warnings. Local profile was backed up for review."
            .to_string();
    }
    if local_dirty {
        if apply_success {
            return "Remote profile was applied. Local profile was backed up for review."
                .to_string();
        }
        return "Remote profile was applied with warnings. Local profile was backed up for review."
            .to_string();
    }
    if apply_success {
        return "Remote profile pulled successfully".to_string();
    }
    "Remote profile pulled with warnings".to_string()
}

pub(crate) fn incident_kind(mode: PullMode) -> &'static str {
    match mode {
        PullMode::Launch => "launch_pull_review",
        PullMode::Manual => "manual_pull_review",
        PullMode::Connect => "connect_pull_review",
    }
}

pub(crate) fn pull_conflict_message(mode: PullMode) -> String {
    match mode {
        PullMode::Launch => "Cloud sync found divergent changes on launch. Local profile was preserved; review the conflict backup before pulling or pushing.".to_string(),
        PullMode::Manual => "Cloud sync found divergent changes. Local profile was preserved; review the conflict backup before pulling or pushing.".to_string(),
        PullMode::Connect => "Cloud sync found divergent changes during setup. Local profile was preserved; review the conflict backup before pulling or pushing.".to_string(),
    }
}

pub(crate) fn pull_local_ahead_message(mode: PullMode) -> String {
    match mode {
        PullMode::Launch => {
            "Local profile has unpushed changes; skipping remote apply until next push".to_string()
        }
        PullMode::Manual => "Local profile has unpushed changes; push to publish them".to_string(),
        PullMode::Connect => "Local profile has unpushed changes relative to remote".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::profile::sync::DEFAULT_PATH;
    use crate::features::profile::sync::{
        GitHubSyncConnection, LocalFolderSyncConnection, SyncConnection, SyncHealth, SyncIncident,
        SyncProviderKind,
    };

    #[test]
    fn build_status_uses_gray_green_yellow_red_model() {
        let base = SyncStateFile::default();
        assert_eq!(build_status(&base).health, SyncHealth::NotConfigured);

        let healthy = SyncStateFile {
            connection: Some(SyncConnection::Github(GitHubSyncConnection {
                gist_id: "abc123def456".to_string(),
                pull_on_launch: true,
                push_on_change: true,
            })),
            ..SyncStateFile::default()
        };
        assert_eq!(build_status(&healthy).health, SyncHealth::Healthy);

        let attention = SyncStateFile {
            incident: Some(SyncIncident {
                kind: "review".to_string(),
                message: "needs review".to_string(),
                backup_file: None,
                created_at: now_rfc3339(),
            }),
            ..healthy.clone()
        };
        assert_eq!(build_status(&attention).health, SyncHealth::Attention);

        let error = SyncStateFile {
            last_error: Some("broken".to_string()),
            ..healthy
        };
        assert_eq!(build_status(&error).health, SyncHealth::Error);
    }

    #[test]
    fn build_status_uses_provider_target_summary() {
        let folder = std::env::temp_dir().join("qol-sync");
        let folder_string = folder.display().to_string();
        let target_string = folder.join(DEFAULT_PATH).display().to_string();
        let state = SyncStateFile {
            connection: Some(SyncConnection::Folder(LocalFolderSyncConnection {
                folder_path: folder_string.clone(),
                path: DEFAULT_PATH.to_string(),
                pull_on_launch: true,
                push_on_change: false,
            })),
            ..SyncStateFile::default()
        };

        let status = build_status(&state);

        assert_eq!(status.provider, Some(SyncProviderKind::Folder));
        assert_eq!(status.provider_label.as_deref(), Some("Folder"));
        assert_eq!(
            status.target_summary.as_deref(),
            Some(target_string.as_str())
        );
        assert_eq!(status.gist_id, None);
        assert_eq!(status.folder_path.as_deref(), Some(folder_string.as_str()));
    }

    #[test]
    fn build_status_health_priority_error_overrides_incident() {
        let state = SyncStateFile {
            connection: Some(SyncConnection::Github(GitHubSyncConnection {
                gist_id: "abc123".to_string(),
                pull_on_launch: true,
                push_on_change: true,
            })),
            incident: Some(SyncIncident {
                kind: "review".to_string(),
                message: "needs review".to_string(),
                backup_file: None,
                created_at: now_rfc3339(),
            }),
            last_error: Some("something broke".to_string()),
            ..SyncStateFile::default()
        };
        assert_eq!(build_status(&state).health, SyncHealth::Error);
    }

    #[test]
    fn build_status_configured_matches_connection_presence() {
        let no_connection = SyncStateFile::default();
        let with_connection = SyncStateFile {
            connection: Some(SyncConnection::Github(GitHubSyncConnection {
                gist_id: "abc123".to_string(),
                pull_on_launch: true,
                push_on_change: true,
            })),
            ..SyncStateFile::default()
        };
        assert!(!build_status(&no_connection).configured);
        assert!(build_status(&with_connection).configured);
    }

    mod prop_tests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(200))]

            #[test]
            fn prop_sanitize_reason_never_empty(input in ".*") {
                let result = sanitize_reason(&input);
                assert!(!result.is_empty());
            }

            #[test]
            fn prop_sanitize_reason_only_alphanumeric_or_hyphen(input in ".*") {
                let result = sanitize_reason(&input);
                if result != "backup" {
                    assert!(result.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-'));
                }
            }

            #[test]
            fn prop_sanitize_reason_preserves_alphanumeric(input in "[a-zA-Z0-9]+") {
                assert_eq!(sanitize_reason(&input), input);
            }

            #[test]
            fn prop_hash_text_is_64_char_hex(input in ".*") {
                let hash = hash_text(&input);
                assert_eq!(hash.len(), 64);
                assert!(hash.chars().all(|ch| ch.is_ascii_hexdigit()));
            }

            #[test]
            fn prop_hash_text_is_deterministic(input in ".*") {
                assert_eq!(hash_text(&input), hash_text(&input));
            }

            #[test]
            fn prop_hash_text_is_lowercase(input in ".*") {
                let hash = hash_text(&input);
                assert_eq!(hash, hash.to_lowercase());
            }
        }
    }

    #[test]
    fn hash_text_different_inputs_produce_different_hashes() {
        let cases = ["", "a", "ab", "abc", "{}", "null", " ", "\n"];
        let hashes: Vec<_> = cases.iter().map(|input| hash_text(input)).collect();
        for (i, a) in hashes.iter().enumerate() {
            for b in &hashes[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn sanitize_reason_edge_cases() {
        let cases = [
            ("", "backup"),
            ("clean", "clean"),
            ("has spaces", "has-spaces"),
            ("special!@#$", "special----"),
            ("123", "123"),
            ("a", "a"),
        ];
        for (input, expected) in cases {
            assert_eq!(sanitize_reason(input), expected, "input: {input:?}");
        }
    }

    #[test]
    fn pull_success_message_covers_all_branches() {
        let cases = [
            (PullMode::Connect, true, true, "applied during setup. Local"),
            (
                PullMode::Connect,
                true,
                false,
                "applied during setup with warnings",
            ),
            (
                PullMode::Manual,
                true,
                true,
                "applied. Local profile was backed up",
            ),
            (
                PullMode::Manual,
                true,
                false,
                "applied with warnings. Local",
            ),
            (PullMode::Launch, false, true, "pulled successfully"),
            (PullMode::Manual, false, false, "pulled with warnings"),
        ];
        for (mode, dirty, success, expected_substring) in cases {
            let msg = pull_success_message(mode, dirty, success);
            assert!(
                msg.contains(expected_substring),
                "mode={mode:?} dirty={dirty} success={success}: {msg:?} missing {expected_substring:?}"
            );
        }
    }

    #[test]
    fn incident_kind_maps_all_modes() {
        assert_eq!(incident_kind(PullMode::Launch), "launch_pull_review");
        assert_eq!(incident_kind(PullMode::Manual), "manual_pull_review");
        assert_eq!(incident_kind(PullMode::Connect), "connect_pull_review");
    }

    #[test]
    fn pull_noop_message_unique_per_mode() {
        let messages: Vec<_> = [PullMode::Launch, PullMode::Manual, PullMode::Connect]
            .iter()
            .map(|mode| pull_noop_message(*mode))
            .collect();
        for (i, a) in messages.iter().enumerate() {
            assert!(!a.is_empty());
            for b in &messages[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }
}
