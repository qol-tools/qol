use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncProviderKind {
    Github,
    Folder,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum SyncConnection {
    Github(GitHubSyncConnection),
    Folder(LocalFolderSyncConnection),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitHubSyncConnection {
    pub gist_id: String,
    #[serde(default = "default_true")]
    pub pull_on_launch: bool,
    #[serde(default = "default_true")]
    pub push_on_change: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LocalFolderSyncConnection {
    pub folder_path: String,
    pub path: String,
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

impl SyncIncidentKind {
    pub fn blocks_auto_sync(self) -> bool {
        matches!(self, Self::Conflict | Self::PushConflict)
    }
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
    pub provider: Option<SyncProviderKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_summary: Option<String>,
    pub health: SyncHealth,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gist_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub pull_on_launch: bool,
    pub push_on_change: bool,
    pub has_github_token: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incident: Option<SyncIncident>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backups_dir: Option<String>,
    pub backup_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_backup_file: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncProviderFieldKey {
    GistId,
    FolderPath,
    Path,
    PullOnLaunch,
    PushOnChange,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncProviderFieldKind {
    Text,
    Password,
    Select,
    Boolean,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncProviderFieldSection {
    Basic,
    Advanced,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SyncProviderFieldDefinition {
    pub key: SyncProviderFieldKey,
    pub label: String,
    pub field_kind: SyncProviderFieldKind,
    pub section: SyncProviderFieldSection,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub full_width: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SyncProviderDefinition {
    pub kind: SyncProviderKind,
    pub label: String,
    pub fields: Vec<SyncProviderFieldDefinition>,
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

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum SyncConnectRequest {
    Github {
        #[serde(default)]
        gist_id: String,
        #[serde(default = "default_true")]
        pull_on_launch: bool,
        #[serde(default = "default_true")]
        push_on_change: bool,
    },
    Folder {
        folder_path: String,
        #[serde(default)]
        path: String,
        #[serde(default = "default_true")]
        pull_on_launch: bool,
        #[serde(default = "default_true")]
        push_on_change: bool,
    },
}

pub(crate) fn default_true() -> bool {
    true
}

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_connect_request_deserializes_github_payload() {
        let request: SyncConnectRequest = serde_json::from_value(serde_json::json!({
            "provider": "github",
            "gist_id": "abc123",
            "pull_on_launch": true,
            "push_on_change": false
        }))
        .unwrap();

        assert_eq!(
            request,
            SyncConnectRequest::Github {
                gist_id: "abc123".to_string(),
                pull_on_launch: true,
                push_on_change: false,
            }
        );
    }

    #[test]
    fn sync_connect_request_deserializes_github_payload_without_gist_id() {
        let request: SyncConnectRequest = serde_json::from_value(serde_json::json!({
            "provider": "github",
        }))
        .unwrap();

        assert_eq!(
            request,
            SyncConnectRequest::Github {
                gist_id: String::new(),
                pull_on_launch: true,
                push_on_change: true,
            }
        );
    }

    #[test]
    fn sync_connect_request_deserializes_folder_payload() {
        let request: SyncConnectRequest = serde_json::from_value(serde_json::json!({
            "provider": "folder",
            "folder_path": std::env::temp_dir().join("qol-sync"),
            "path": "profiles/main.json",
            "pull_on_launch": false,
            "push_on_change": true
        }))
        .unwrap();

        assert_eq!(
            request,
            SyncConnectRequest::Folder {
                folder_path: std::env::temp_dir().join("qol-sync").display().to_string(),
                path: "profiles/main.json".to_string(),
                pull_on_launch: false,
                push_on_change: true,
            }
        );
    }
}
