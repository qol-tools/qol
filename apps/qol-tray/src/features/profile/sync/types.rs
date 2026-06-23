use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct SyncConnectRequest {
    #[serde(default)]
    pub repo_url: Option<String>,
    #[serde(default = "default_true")]
    pub auto_create: bool,
    #[serde(default = "default_true")]
    pub pull_on_launch: bool,
    #[serde(default = "default_true")]
    pub push_on_change: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncIncidentKind {
    Conflict,
    PushConflict,
    LaunchPullReview,
    ManualPullReview,
    ConnectPullReview,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncIncident {
    pub kind: SyncIncidentKind,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backup_file: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncHealth {
    NotConfigured,
    Healthy,
    Attention,
    Error,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SyncStatus {
    pub configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_url: Option<String>,
    pub auto_created: bool,
    pub health: SyncHealth,
    pub pull_on_launch: bool,
    pub push_on_change: bool,
    pub has_github_token: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incident: Option<SyncIncident>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backups_dir: Option<String>,
    pub backup_count: usize,
    pub conflict_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_backup_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SyncBackupEntry {
    pub file_name: String,
    pub created_at: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SyncBackupPreview {
    pub file_name: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SyncActionResult {
    pub message: String,
    pub applied_remote: bool,
    pub status: SyncStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvableConflict {
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugin: Option<String>,
    pub key_path: String,
    pub local: Value,
    pub remote: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_edited: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_edited: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Mine,
    Remote,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct ConflictChoice {
    pub file: String,
    pub key_path: String,
    pub side: Side,
}

pub(crate) fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_request_defaults_to_auto_create_and_no_url() {
        let request: SyncConnectRequest = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(
            request,
            SyncConnectRequest {
                repo_url: None,
                auto_create: true,
                pull_on_launch: true,
                push_on_change: true,
            }
        );
    }

    #[test]
    fn connect_request_accepts_explicit_repo_url() {
        let request: SyncConnectRequest = serde_json::from_value(serde_json::json!({
            "repo_url": "https://github.com/me/qol-tray-profiles",
            "auto_create": false,
            "pull_on_launch": false,
            "push_on_change": false,
        }))
        .unwrap();
        assert_eq!(
            request,
            SyncConnectRequest {
                repo_url: Some("https://github.com/me/qol-tray-profiles".to_string()),
                auto_create: false,
                pull_on_launch: false,
                push_on_change: false,
            }
        );
    }
}
