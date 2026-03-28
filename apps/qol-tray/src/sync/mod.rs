use anyhow::{anyhow, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;

const DEFAULT_BRANCH: &str = "main";
const DEFAULT_PATH: &str = "qol-tray/profile.json";
const DEFAULT_COMMIT_MESSAGE: &str = "chore: sync qol-tray profile";
const AUTO_PUSH_INTERVAL_SECS: u64 = 3;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncProviderKind {
    Github,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum SyncConnection {
    Github(GitHubSyncConnection),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GitHubSyncConnection {
    pub repo_url: String,
    pub branch: String,
    pub path: String,
    pub commit_message: String,
    #[serde(default = "default_true")]
    pub pull_on_launch: bool,
    #[serde(default = "default_true")]
    pub push_on_change: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct SyncStateFile {
    #[serde(default)]
    connection: Option<SyncConnection>,
    #[serde(default)]
    remote_revision: Option<String>,
    #[serde(default)]
    last_synced_hash: Option<String>,
    #[serde(default)]
    last_sync_at: Option<String>,
    #[serde(default)]
    incident: Option<SyncIncident>,
    #[serde(default)]
    last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SyncIncident {
    pub kind: String,
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
    pub health: SyncHealth,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_message: Option<String>,
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SyncActionResult {
    pub message: String,
    pub applied_remote: bool,
    pub status: SyncStatus,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SyncConnectRequest {
    #[serde(default = "default_provider_kind")]
    pub provider: SyncProviderKind,
    #[serde(default)]
    pub token: String,
    pub repo_url: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub commit_message: String,
    #[serde(default = "default_true")]
    pub pull_on_launch: bool,
    #[serde(default = "default_true")]
    pub push_on_change: bool,
}

pub struct SyncService {
    client: reqwest::Client,
    plugins_dir: PathBuf,
    state: Mutex<SyncStateFile>,
    operation_lock: AsyncMutex<()>,
}

#[derive(Debug, Clone)]
struct RemoteDocument {
    revision: String,
    content: String,
}

#[derive(Debug)]
enum ProviderError {
    Auth(String),
    Conflict(String),
    Invalid(String),
    Upstream(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auth(message) => write!(formatter, "{}", message),
            Self::Conflict(message) => write!(formatter, "{}", message),
            Self::Invalid(message) => write!(formatter, "{}", message),
            Self::Upstream(message) => write!(formatter, "{}", message),
        }
    }
}

impl std::error::Error for ProviderError {}

#[derive(Clone, Copy)]
enum PullMode {
    Launch,
    Manual,
    Connect,
}

impl SyncService {
    pub fn new(plugins_dir: PathBuf) -> Result<Self> {
        ensure_sync_dirs()?;
        Ok(Self {
            client: reqwest::Client::new(),
            plugins_dir,
            state: Mutex::new(load_state_file().unwrap_or_default()),
            operation_lock: AsyncMutex::new(()),
        })
    }

    pub fn auto_push_interval() -> Duration {
        Duration::from_secs(AUTO_PUSH_INTERVAL_SECS)
    }

    pub fn status(&self) -> SyncStatus {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        build_status(&state)
    }

    pub async fn connect(&self, request: SyncConnectRequest) -> Result<SyncActionResult> {
        let _operation = self.operation_lock.lock().await;
        let connection = request.to_connection()?;
        validate_connection(&connection)?;
        let token = self.connect_token(&request, &connection)?;
        if let Err(error) = validate_token(&token, &connection).await {
            return self.return_error(anyhow!(error.to_string()));
        }
        store_token(&token, &connection)?;

        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.connection = Some(connection.clone());
            state.remote_revision = None;
            state.last_synced_hash = None;
            state.last_sync_at = None;
            state.incident = None;
            state.last_error = None;
            save_state_file(&state)?;
        }

        let remote = match fetch_remote_document(&self.client, &connection, &token).await {
            Ok(remote) => remote,
            Err(error) => return self.return_error(anyhow!(error)),
        };
        if let Some(remote) = remote {
            return match self.apply_remote_document(remote, PullMode::Connect).await {
                Ok(result) => Ok(result),
                Err(error) => self.return_error(error),
            };
        }

        match self.push_current_document(Some(connection)).await {
            Ok(result) => Ok(result),
            Err(error) => self.return_error(error),
        }
    }

    pub async fn disconnect(&self) -> Result<SyncActionResult> {
        let _operation = self.operation_lock.lock().await;
        let connection = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.connection.clone()
        };
        delete_token(connection.as_ref())?;
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *state = SyncStateFile::default();
            save_state_file(&state)?;
        }
        Ok(SyncActionResult {
            message: "Cloud sync disconnected".to_string(),
            applied_remote: false,
            status: self.status(),
        })
    }

    pub async fn pull_on_launch(&self) -> Result<SyncActionResult> {
        let should_pull = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            pull_on_launch_enabled(state.connection.as_ref())
        };
        if !should_pull {
            return Ok(self.noop("Cloud sync launch pull is disabled"));
        }
        match self.pull(PullMode::Launch).await {
            Ok(result) => Ok(result),
            Err(error) => self.return_error(error),
        }
    }

    pub async fn manual_pull(&self) -> Result<SyncActionResult> {
        match self.pull(PullMode::Manual).await {
            Ok(result) => Ok(result),
            Err(error) => self.return_error(error),
        }
    }

    pub async fn manual_push(&self) -> Result<SyncActionResult> {
        let _operation = self.operation_lock.lock().await;
        match self.push_current_document(None).await {
            Ok(result) => Ok(result),
            Err(error) => self.return_error(error),
        }
    }

    pub async fn auto_push_if_dirty(&self) -> Result<SyncActionResult> {
        let enabled = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.incident.is_some() {
                return Ok(self.noop("Cloud sync is waiting for acknowledgement"));
            }
            if state.last_error.is_some() {
                return Ok(self.noop("Cloud sync is blocked by an error"));
            }
            push_on_change_enabled(state.connection.as_ref())
        };
        if !enabled {
            return Ok(self.noop("Cloud sync auto-push is disabled"));
        }

        let _operation = self.operation_lock.lock().await;
        let current_hash = self.current_sync_hash()?;
        {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.last_synced_hash.as_deref() == Some(current_hash.as_str()) {
                return Ok(self.noop("Cloud sync is already up to date"));
            }
        }

        match self.push_current_document(None).await {
            Ok(result) => Ok(result),
            Err(error) => self.return_error(error),
        }
    }

    pub async fn acknowledge_incident(&self) -> Result<SyncActionResult> {
        let _operation = self.operation_lock.lock().await;
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.incident = None;
            save_state_file(&state)?;
        }
        Ok(SyncActionResult {
            message: "Sync review acknowledged".to_string(),
            applied_remote: false,
            status: self.status(),
        })
    }

    pub fn open_backups_dir(&self) -> Result<()> {
        ensure_sync_dirs()?;
        open_dir(&crate::paths::sync_backups_dir()?)
    }

    fn noop(&self, message: &str) -> SyncActionResult {
        SyncActionResult {
            message: message.to_string(),
            applied_remote: false,
            status: self.status(),
        }
    }

    fn connect_token(
        &self,
        request: &SyncConnectRequest,
        connection: &SyncConnection,
    ) -> Result<String> {
        let trimmed = request.token.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
        current_token(connection).context("GitHub PAT is required")
    }

    async fn pull(&self, mode: PullMode) -> Result<SyncActionResult> {
        let _operation = self.operation_lock.lock().await;
        let connection = self.current_connection()?;
        let token = current_token(&connection)?;
        let remote = fetch_remote_document(&self.client, &connection, &token).await?;
        let Some(remote) = remote else {
            return self.fail_error("Remote profile file was not found");
        };
        self.apply_remote_document(remote, mode).await
    }

    async fn apply_remote_document(
        &self,
        remote: RemoteDocument,
        mode: PullMode,
    ) -> Result<SyncActionResult> {
        let local_json = self.current_sync_document_json()?;
        let local_hash = hash_text(&local_json);
        let remote_hash = hash_text(&remote.content);
        if remote_hash == local_hash {
            self.mark_synced(remote.revision, remote_hash)?;
            let message = pull_noop_message(mode);
            return Ok(SyncActionResult {
                message,
                applied_remote: false,
                status: self.status(),
            });
        }

        let local_dirty = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.last_synced_hash.as_deref() != Some(local_hash.as_str())
        };

        let backup_file = if local_dirty {
            Some(self.write_backup_file("remote-applied")?)
        } else {
            None
        };

        let bundle: crate::profile::ProfileImportBundle =
            serde_json::from_str(&remote.content).context("Remote profile JSON is invalid")?;
        let result = crate::profile::apply_import_bundle(&self.plugins_dir, &bundle).await?;
        let message = pull_success_message(mode, local_dirty, result.success);

        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.remote_revision = Some(remote.revision);
            state.last_synced_hash = Some(remote_hash);
            state.last_sync_at = Some(now_rfc3339());
            state.last_error = None;
            if local_dirty || !result.success {
                state.incident = Some(SyncIncident {
                    kind: incident_kind(mode).to_string(),
                    message: message.clone(),
                    backup_file: backup_file.as_ref().map(|path| filename_string(path)),
                    created_at: now_rfc3339(),
                });
            }
            save_state_file(&state)?;
        }

        Ok(SyncActionResult {
            message,
            applied_remote: true,
            status: self.status(),
        })
    }

    async fn push_current_document(
        &self,
        override_connection: Option<SyncConnection>,
    ) -> Result<SyncActionResult> {
        let connection = match override_connection {
            Some(connection) => connection,
            None => self.current_connection()?,
        };
        let token = current_token(&connection)?;
        let local_json = self.current_sync_document_json()?;
        let local_hash = hash_text(&local_json);
        let remote = fetch_remote_document(&self.client, &connection, &token).await?;
        if let Some(remote) = &remote {
            let remote_hash = hash_text(&remote.content);
            if remote_hash == local_hash {
                self.mark_synced(remote.revision.clone(), local_hash)?;
                return Ok(SyncActionResult {
                    message: "Cloud sync is already up to date".to_string(),
                    applied_remote: false,
                    status: self.status(),
                });
            }
        }

        let remote_revision = remote.as_ref().map(|document| document.revision.clone());
        let stored_revision = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.remote_revision.clone()
        };
        if remote_revision != stored_revision && stored_revision.is_some() {
            let remote = remote.context("Remote profile changed before push")?;
            return self.resolve_push_conflict(connection, remote).await;
        }

        let next_revision = match push_remote_document(
            &self.client,
            &connection,
            &token,
            &local_json,
            remote_revision.as_deref(),
        )
        .await
        {
            Ok(revision) => revision,
            Err(ProviderError::Conflict(_)) => {
                let remote = fetch_remote_document(&self.client, &connection, &token)
                    .await
                    .map_err(|error| anyhow!(error))?
                    .context("Remote profile changed before push")?;
                return self.resolve_push_conflict(connection, remote).await;
            }
            Err(error) => return Err(anyhow!(error)),
        };

        self.mark_synced(next_revision, local_hash)?;
        Ok(SyncActionResult {
            message: "Cloud sync pushed local changes".to_string(),
            applied_remote: false,
            status: self.status(),
        })
    }

    async fn resolve_push_conflict(
        &self,
        connection: SyncConnection,
        remote: RemoteDocument,
    ) -> Result<SyncActionResult> {
        let backup_file = self.write_backup_file("push-conflict")?;
        let bundle: crate::profile::ProfileImportBundle =
            serde_json::from_str(&remote.content).context("Remote profile JSON is invalid")?;
        let result = crate::profile::apply_import_bundle(&self.plugins_dir, &bundle).await?;
        let remote_hash = hash_text(&remote.content);
        let message = if result.success {
            "Remote profile changed first. Local profile was backed up and remote changes were applied."
        } else {
            "Remote profile changed first. Local profile was backed up and remote changes were applied with warnings."
        };

        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.connection = Some(connection);
            state.remote_revision = Some(remote.revision);
            state.last_synced_hash = Some(remote_hash);
            state.last_sync_at = Some(now_rfc3339());
            state.last_error = None;
            state.incident = Some(SyncIncident {
                kind: "push_conflict".to_string(),
                message: message.to_string(),
                backup_file: Some(filename_string(&backup_file)),
                created_at: now_rfc3339(),
            });
            save_state_file(&state)?;
        }

        Ok(SyncActionResult {
            message: message.to_string(),
            applied_remote: true,
            status: self.status(),
        })
    }

    fn current_connection(&self) -> Result<SyncConnection> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(connection) = &state.connection else {
            anyhow::bail!("Cloud sync is not configured");
        };
        Ok(connection.clone())
    }

    fn current_sync_document_json(&self) -> Result<String> {
        let plugins = crate::profile::load_plugins_lock()
            .map(|lock| lock.plugins)
            .unwrap_or_default();
        crate::profile::build_sync_document_json(plugins)
    }

    fn current_sync_hash(&self) -> Result<String> {
        Ok(hash_text(&self.current_sync_document_json()?))
    }

    fn write_backup_file(&self, reason: &str) -> Result<PathBuf> {
        ensure_sync_dirs()?;
        let filename = format!(
            "{}-{}.json",
            chrono::Local::now().format("%Y%m%d-%H%M%S"),
            sanitize_reason(reason)
        );
        let path = crate::paths::sync_backups_dir()?.join(filename);
        let plugins = crate::profile::load_plugins_lock()
            .map(|lock| lock.plugins)
            .unwrap_or_default();
        let content = crate::profile::build_export_bundle_json(now_rfc3339(), plugins)?;
        crate::file_io::atomic_write(&path, content.as_bytes())?;
        Ok(path)
    }

    fn mark_synced(&self, remote_revision: String, local_hash: String) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.remote_revision = Some(remote_revision);
        state.last_synced_hash = Some(local_hash);
        state.last_sync_at = Some(now_rfc3339());
        state.last_error = None;
        save_state_file(&state)?;
        Ok(())
    }

    fn return_error<T>(&self, error: anyhow::Error) -> Result<T> {
        log::error!("Cloud sync failed: {error:#}");
        if let Err(write_error) = self.store_last_error(&error.to_string()) {
            log::error!("Failed to persist cloud sync error state: {write_error:#}");
        }
        Err(error)
    }

    fn store_last_error(&self, message: &str) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.connection.is_none() {
            return Ok(());
        }
        state.last_error = Some(message.to_string());
        save_state_file(&state)
    }

    fn fail_error(&self, message: &str) -> Result<SyncActionResult> {
        log::error!("Cloud sync error: {}", message);
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.last_error = Some(message.to_string());
        save_state_file(&state)?;
        Ok(SyncActionResult {
            message: message.to_string(),
            applied_remote: false,
            status: build_status(&state),
        })
    }
}

impl SyncConnectRequest {
    fn to_connection(&self) -> Result<SyncConnection> {
        if self.provider != SyncProviderKind::Github {
            anyhow::bail!("Unsupported sync provider");
        }
        Ok(SyncConnection::Github(GitHubSyncConnection {
            repo_url: normalize_repo_url(&self.repo_url)?,
            branch: normalize_branch(&self.branch)?,
            path: normalize_path(&self.path)?,
            commit_message: normalize_commit_message(&self.commit_message),
            pull_on_launch: self.pull_on_launch,
            push_on_change: self.push_on_change,
        }))
    }
}

fn build_status(state: &SyncStateFile) -> SyncStatus {
    let backups_dir = crate::paths::sync_backups_dir().ok();
    let backup_files = list_backup_files(backups_dir.as_deref());
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
        provider: connection.map(connection_provider_kind),
        provider_label: connection.map(connection_provider_label),
        health,
        repo_url: connection.map(connection_repo_url),
        branch: connection.map(connection_branch),
        path: connection.map(connection_path),
        commit_message: connection.map(connection_commit_message),
        pull_on_launch: pull_on_launch_enabled(connection),
        push_on_change: push_on_change_enabled(connection),
        has_github_token: crate::credentials::github_bearer_token().is_some(),
        last_sync_at: state.last_sync_at.clone(),
        incident: state.incident.clone(),
        last_error: state.last_error.clone(),
        backups_dir: backups_dir.map(|path| path.display().to_string()),
        backup_count: backup_files.len(),
        latest_backup_file: backup_files.last().cloned(),
    }
}

fn connection_provider_kind(connection: &SyncConnection) -> SyncProviderKind {
    match connection {
        SyncConnection::Github(_) => SyncProviderKind::Github,
    }
}

fn connection_provider_label(connection: &SyncConnection) -> String {
    match connection {
        SyncConnection::Github(_) => "GitHub".to_string(),
    }
}

fn connection_repo_url(connection: &SyncConnection) -> String {
    match connection {
        SyncConnection::Github(connection) => connection.repo_url.clone(),
    }
}

fn connection_branch(connection: &SyncConnection) -> String {
    match connection {
        SyncConnection::Github(connection) => connection.branch.clone(),
    }
}

fn connection_path(connection: &SyncConnection) -> String {
    match connection {
        SyncConnection::Github(connection) => connection.path.clone(),
    }
}

fn connection_commit_message(connection: &SyncConnection) -> String {
    match connection {
        SyncConnection::Github(connection) => connection.commit_message.clone(),
    }
}

fn pull_on_launch_enabled(connection: Option<&SyncConnection>) -> bool {
    let Some(connection) = connection else {
        return false;
    };
    match connection {
        SyncConnection::Github(connection) => connection.pull_on_launch,
    }
}

fn push_on_change_enabled(connection: Option<&SyncConnection>) -> bool {
    let Some(connection) = connection else {
        return false;
    };
    match connection {
        SyncConnection::Github(connection) => connection.push_on_change,
    }
}

fn current_token(connection: &SyncConnection) -> Result<String> {
    match connection {
        SyncConnection::Github(_) => crate::credentials::github_bearer_token()
            .ok_or_else(|| anyhow!("GitHub credential is not configured")),
    }
}

async fn validate_token(token: &str, connection: &SyncConnection) -> Result<()> {
    match connection {
        SyncConnection::Github(_) => crate::features::plugin_store::github::validate_token(token)
            .await
            .map_err(|error| anyhow!(error.to_string())),
    }
}

fn store_token(token: &str, connection: &SyncConnection) -> Result<()> {
    match connection {
        SyncConnection::Github(_) => crate::credentials::store_github_token(token),
    }
}

fn delete_token(connection: Option<&SyncConnection>) -> Result<()> {
    let Some(connection) = connection else {
        return Ok(());
    };
    match connection {
        SyncConnection::Github(_) => crate::credentials::delete_github_token(),
    }
}

fn ensure_sync_dirs() -> Result<()> {
    std::fs::create_dir_all(crate::paths::sync_dir()?)?;
    std::fs::create_dir_all(crate::paths::sync_backups_dir()?)?;
    Ok(())
}

fn load_state_file() -> Result<SyncStateFile> {
    let path = crate::paths::sync_state_path()?;
    if !path.exists() {
        return Ok(SyncStateFile::default());
    }
    crate::file_io::read_json(&path)
}

fn save_state_file(state: &SyncStateFile) -> Result<()> {
    ensure_sync_dirs()?;
    crate::file_io::write_pretty_json(&crate::paths::sync_state_path()?, state)
}

fn list_backup_files(dir: Option<&Path>) -> Vec<String> {
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
            Some(filename_string(&path))
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn filename_string(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

fn sanitize_reason(reason: &str) -> String {
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

fn hash_text(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339()
}

fn default_true() -> bool {
    true
}

fn default_provider_kind() -> SyncProviderKind {
    SyncProviderKind::Github
}

fn normalize_repo_url(repo_url: &str) -> Result<String> {
    let trimmed = repo_url.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Repo URL cannot be empty");
    }
    parse_github_repo(trimmed)?;
    Ok(trimmed.to_string())
}

fn normalize_branch(branch: &str) -> Result<String> {
    let trimmed = branch.trim();
    if trimmed.is_empty() {
        return Ok(DEFAULT_BRANCH.to_string());
    }
    if !is_safe_branch(trimmed) {
        anyhow::bail!("Invalid branch");
    }
    Ok(trimmed.to_string())
}

fn normalize_path(path: &str) -> Result<String> {
    let trimmed = path.trim().trim_start_matches('/');
    if trimmed.is_empty() {
        return Ok(DEFAULT_PATH.to_string());
    }
    if !is_safe_remote_path(trimmed) {
        anyhow::bail!("Invalid remote path");
    }
    Ok(trimmed.to_string())
}

fn normalize_commit_message(message: &str) -> String {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return DEFAULT_COMMIT_MESSAGE.to_string();
    }
    trimmed.to_string()
}

fn validate_connection(connection: &SyncConnection) -> Result<()> {
    match connection {
        SyncConnection::Github(connection) => {
            parse_github_repo(&connection.repo_url)?;
            if !is_safe_branch(&connection.branch) {
                anyhow::bail!("Invalid branch");
            }
            if !is_safe_remote_path(&connection.path) {
                anyhow::bail!("Invalid remote path");
            }
            Ok(())
        }
    }
}

fn parse_github_repo(repo_url: &str) -> Result<(String, String)> {
    if let Some(rest) = repo_url.strip_prefix("https://github.com/") {
        return parse_owner_repo(rest);
    }
    if let Some(rest) = repo_url.strip_prefix("http://github.com/") {
        return parse_owner_repo(rest);
    }
    if let Some((_, rest)) = repo_url.split_once(':') {
        if repo_url.starts_with("git@") {
            return parse_owner_repo(rest);
        }
    }
    if let Some((_, rest)) = repo_url.split_once("ssh://git@") {
        let Some((_, path)) = rest.split_once('/') else {
            anyhow::bail!("Repo URL must include owner and repo");
        };
        return parse_owner_repo(path);
    }
    anyhow::bail!("Repo URL must point to a GitHub repository")
}

fn parse_owner_repo(raw: &str) -> Result<(String, String)> {
    let trimmed = raw.trim_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let mut parts = trimmed.split('/');
    let owner = parts.next().unwrap_or_default();
    let repo = parts.next().unwrap_or_default();
    if parts.next().is_some() {
        anyhow::bail!("Repo URL must only include owner and repo");
    }
    if !is_safe_repo_part(owner) || !is_safe_repo_part(repo) {
        anyhow::bail!("Repo URL contains an invalid owner or repo");
    }
    Ok((owner.to_string(), repo.to_string()))
}

fn is_safe_repo_part(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
}

fn is_safe_branch(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    if value.contains("..") || value.contains('\\') {
        return false;
    }
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '/' || ch == '.')
}

fn is_safe_remote_path(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    if value.contains("..") || value.contains('\\') || value.ends_with('/') {
        return false;
    }
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '/' || ch == '.')
}

async fn fetch_remote_document(
    client: &reqwest::Client,
    connection: &SyncConnection,
    token: &str,
) -> std::result::Result<Option<RemoteDocument>, ProviderError> {
    match connection {
        SyncConnection::Github(connection) => {
            let (owner, repo) = parse_github_repo(&connection.repo_url)
                .map_err(|error| ProviderError::Invalid(error.to_string()))?;
            let url = format!(
                "https://api.github.com/repos/{owner}/{repo}/contents/{}",
                connection.path
            );
            let response = client
                .get(url)
                .query(&[("ref", connection.branch.as_str())])
                .header("User-Agent", "qol-tray")
                .header("Authorization", format!("Bearer {}", token))
                .send()
                .await
                .map_err(|error| ProviderError::Upstream(error.to_string()))?;

            let status = response.status();
            if status == reqwest::StatusCode::NOT_FOUND {
                return Ok(None);
            }
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                let body = response.text().await.unwrap_or_default();
                return Err(ProviderError::Auth(format!(
                    "GitHub authentication failed: {} {}",
                    status, body
                )));
            }
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!(
                    "GitHub returned {}: {}",
                    status, body
                )));
            }

            #[derive(Deserialize)]
            struct ResponseBody {
                sha: String,
                content: String,
                encoding: String,
            }

            let body: ResponseBody = response
                .json()
                .await
                .map_err(|error| ProviderError::Upstream(error.to_string()))?;
            if body.encoding != "base64" {
                return Err(ProviderError::Invalid(format!(
                    "Unsupported GitHub content encoding: {}",
                    body.encoding
                )));
            }
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(body.content.replace('\n', ""))
                .map_err(|error| ProviderError::Invalid(error.to_string()))?;
            let content = String::from_utf8(decoded)
                .map_err(|error| ProviderError::Invalid(error.to_string()))?;
            Ok(Some(RemoteDocument {
                revision: body.sha,
                content,
            }))
        }
    }
}

async fn push_remote_document(
    client: &reqwest::Client,
    connection: &SyncConnection,
    token: &str,
    content: &str,
    remote_revision: Option<&str>,
) -> std::result::Result<String, ProviderError> {
    match connection {
        SyncConnection::Github(connection) => {
            let (owner, repo) = parse_github_repo(&connection.repo_url)
                .map_err(|error| ProviderError::Invalid(error.to_string()))?;
            let url = format!(
                "https://api.github.com/repos/{owner}/{repo}/contents/{}",
                connection.path
            );

            #[derive(Serialize)]
            struct RequestBody<'a> {
                message: &'a str,
                content: String,
                branch: &'a str,
                #[serde(skip_serializing_if = "Option::is_none")]
                sha: Option<&'a str>,
            }

            #[derive(Deserialize)]
            struct ResponseBody {
                content: ContentBody,
            }

            #[derive(Deserialize)]
            struct ContentBody {
                sha: String,
            }

            let response = client
                .put(url)
                .header("User-Agent", "qol-tray")
                .header("Authorization", format!("Bearer {}", token))
                .json(&RequestBody {
                    message: &connection.commit_message,
                    content: base64::engine::general_purpose::STANDARD.encode(content.as_bytes()),
                    branch: &connection.branch,
                    sha: remote_revision,
                })
                .send()
                .await
                .map_err(|error| ProviderError::Upstream(error.to_string()))?;

            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED
                || status == reqwest::StatusCode::FORBIDDEN
            {
                let body = response.text().await.unwrap_or_default();
                return Err(ProviderError::Auth(format!(
                    "GitHub authentication failed: {} {}",
                    status, body
                )));
            }
            if status == reqwest::StatusCode::CONFLICT {
                let body = response.text().await.unwrap_or_default();
                return Err(ProviderError::Conflict(format!(
                    "GitHub reported a sync conflict: {}",
                    body
                )));
            }
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(ProviderError::Upstream(format!(
                    "GitHub returned {}: {}",
                    status, body
                )));
            }

            let body: ResponseBody = response
                .json()
                .await
                .map_err(|error| ProviderError::Upstream(error.to_string()))?;
            Ok(body.content.sha)
        }
    }
}

fn pull_noop_message(mode: PullMode) -> String {
    match mode {
        PullMode::Launch => "Cloud sync checked remote profile on launch".to_string(),
        PullMode::Manual => "Cloud sync is already up to date".to_string(),
        PullMode::Connect => "Cloud sync connected to an already-synced profile".to_string(),
    }
}

fn pull_success_message(mode: PullMode, local_dirty: bool, apply_success: bool) -> String {
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

fn incident_kind(mode: PullMode) -> &'static str {
    match mode {
        PullMode::Launch => "launch_pull_review",
        PullMode::Manual => "manual_pull_review",
        PullMode::Connect => "connect_pull_review",
    }
}

fn open_dir(dir: &Path) -> Result<()> {
    if !dir.exists() {
        anyhow::bail!("Directory does not exist");
    }

    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(dir).spawn();
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(dir).spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(dir).spawn();

    result.map(|_| ()).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_github_repo_cases() {
        let cases = vec![
            (
                "https://github.com/qol-tools/qol-tray",
                Some(("qol-tools", "qol-tray")),
            ),
            (
                "https://github.com/qol-tools/qol-tray.git",
                Some(("qol-tools", "qol-tray")),
            ),
            (
                "git@github.com:qol-tools/qol-tray.git",
                Some(("qol-tools", "qol-tray")),
            ),
            (
                "git@github-priv:qol-tools/qol-tray.git",
                Some(("qol-tools", "qol-tray")),
            ),
            (
                "ssh://git@github.com/qol-tools/qol-tray.git",
                Some(("qol-tools", "qol-tray")),
            ),
            ("https://example.com/qol-tools/qol-tray", None),
        ];

        for (input, expected) in cases {
            let parsed = parse_github_repo(input).ok();
            let expected = expected.map(|(owner, repo)| (owner.to_string(), repo.to_string()));
            assert_eq!(parsed, expected, "input: {}", input);
        }
    }

    #[test]
    fn normalize_path_cases() {
        let cases = vec![
            ("", Some(DEFAULT_PATH.to_string())),
            ("qol/profile.json", Some("qol/profile.json".to_string())),
            ("/qol/profile.json", Some("qol/profile.json".to_string())),
            ("../bad.json", None),
            ("qol\\bad.json", None),
        ];

        for (input, expected) in cases {
            let actual = normalize_path(input).ok();
            assert_eq!(actual, expected, "input: {}", input);
        }
    }

    #[test]
    fn build_status_uses_gray_green_yellow_red_model() {
        let base = SyncStateFile::default();
        assert_eq!(build_status(&base).health, SyncHealth::NotConfigured);

        let healthy = SyncStateFile {
            connection: Some(SyncConnection::Github(GitHubSyncConnection {
                repo_url: "https://github.com/qol-tools/qol-tray".to_string(),
                branch: DEFAULT_BRANCH.to_string(),
                path: DEFAULT_PATH.to_string(),
                commit_message: DEFAULT_COMMIT_MESSAGE.to_string(),
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
    fn sync_connect_request_normalizes_defaults() {
        let request = SyncConnectRequest {
            provider: SyncProviderKind::Github,
            token: String::new(),
            repo_url: " https://github.com/qol-tools/qol-tray.git ".to_string(),
            branch: String::new(),
            path: " /profiles/main.json ".to_string(),
            commit_message: String::new(),
            pull_on_launch: true,
            push_on_change: false,
        };

        let connection = request.to_connection().unwrap();

        assert_eq!(
            connection,
            SyncConnection::Github(GitHubSyncConnection {
                repo_url: "https://github.com/qol-tools/qol-tray.git".to_string(),
                branch: DEFAULT_BRANCH.to_string(),
                path: "profiles/main.json".to_string(),
                commit_message: DEFAULT_COMMIT_MESSAGE.to_string(),
                pull_on_launch: true,
                push_on_change: false,
            })
        );
    }

    #[test]
    fn sync_connect_request_rejects_invalid_remote_path() {
        let request = SyncConnectRequest {
            provider: SyncProviderKind::Github,
            token: String::new(),
            repo_url: "https://github.com/qol-tools/qol-tray.git".to_string(),
            branch: "main".to_string(),
            path: "../profile.json".to_string(),
            commit_message: String::new(),
            pull_on_launch: true,
            push_on_change: true,
        };

        assert!(request.to_connection().is_err());
    }
}
