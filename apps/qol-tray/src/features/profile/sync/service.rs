use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;

use super::platform::{open_dir, open_path};
use super::providers::{
    ensure_profile_gist, normalize_folder_path, normalize_path, sync_provider_definitions,
    validate_github_token, ProviderError, RemoteDocument,
};
use super::resolve::{resolve_sync_action, SyncAction};
use super::state::{
    backup_file_path, build_status, ensure_sync_dirs, filename_string, hash_text, incident_kind,
    list_backup_entries, load_state_file, now_rfc3339, pull_conflict_message,
    pull_local_ahead_message, pull_noop_message, pull_success_message, sanitize_reason,
    save_state_file, PullMode, SyncStateFile,
};
use super::types::{
    SyncActionResult, SyncBackupEntry, SyncBackupPreview, SyncConnectRequest, SyncConnection,
    SyncProviderDefinition,
};
use super::AUTO_PUSH_INTERVAL_SECS;

impl SyncConnectRequest {
    async fn prepare(self, service: &SyncService) -> Result<PreparedSyncConnectRequest> {
        match self {
            Self::Github {
                gist_id,
                pull_on_launch,
                push_on_change,
            } => {
                service
                    .prepare_github_connect(gist_id, pull_on_launch, push_on_change)
                    .await
            }
            Self::Folder {
                folder_path,
                path,
                pull_on_launch,
                push_on_change,
            } => service.prepare_folder_connect(folder_path, path, pull_on_launch, push_on_change),
        }
    }
}

#[derive(Debug, Clone)]
struct PreparedSyncConnectRequest {
    connection: SyncConnection,
    github_token: Option<String>,
}

#[derive(Debug, Clone)]
struct SyncOperationOutcome {
    message: String,
    applied_remote: bool,
    remote_revision: String,
    last_synced_hash_update: Option<String>,
    incident: Option<super::types::SyncIncident>,
}

pub struct SyncService {
    client: reqwest::Client,
    plugins_dir: PathBuf,
    state: Mutex<SyncStateFile>,
    operation_lock: AsyncMutex<()>,
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

    pub fn providers(&self) -> Vec<SyncProviderDefinition> {
        sync_provider_definitions()
    }

    pub fn auto_push_interval() -> Duration {
        Duration::from_secs(AUTO_PUSH_INTERVAL_SECS)
    }

    pub fn status(&self) -> super::types::SyncStatus {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        build_status(&state)
    }

    pub async fn connect(&self, request: SyncConnectRequest) -> Result<SyncActionResult> {
        let _operation = self.operation_lock.lock().await;
        let prepared = match request.prepare(self).await {
            Ok(prepared) => prepared,
            Err(error) => return self.return_unpersisted_error(error),
        };
        let connection = prepared.connection.clone();
        if let Err(error) = connection.validate() {
            return self.return_unpersisted_error(error);
        }

        let remote = match connection
            .fetch_remote_document(&self.client, prepared.github_token.as_deref())
            .await
        {
            Ok(remote) => remote,
            Err(error) => return self.return_unpersisted_error(anyhow!(error)),
        };
        if let Some(remote) = remote {
            return match self.apply_remote_document(remote, PullMode::Connect).await {
                Ok(outcome) => self.finalize_connect(prepared, outcome),
                Err(error) => self.return_unpersisted_error(error),
            };
        }

        match self
            .push_current_document(Some(connection), prepared.github_token.as_deref())
            .await
        {
            Ok(outcome) => self.finalize_connect(prepared, outcome),
            Err(error) => self.return_unpersisted_error(error),
        }
    }

    pub async fn bootstrap_github_connect(&self) -> Result<SyncActionResult> {
        let token = crate::features::github_auth::oauth_access_token()
            .context("GitHub account is not connected")?;
        let gist_id = ensure_profile_gist(&self.client, &token)
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        self.connect(SyncConnectRequest::Github {
            gist_id,
            pull_on_launch: true,
            push_on_change: true,
        })
        .await
    }

    pub async fn disconnect(&self) -> Result<SyncActionResult> {
        let _operation = self.operation_lock.lock().await;
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
            state
                .connection
                .as_ref()
                .map(SyncConnection::pull_on_launch)
                .unwrap_or(false)
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
        match self.push_current_document(None, None).await {
            Ok(outcome) => {
                self.persist_operation_state(None, &outcome, None)?;
                Ok(self.operation_result(&outcome))
            }
            Err(error) => self.return_error(error),
        }
    }

    pub async fn auto_push_if_dirty(&self) -> Result<SyncActionResult> {
        let enabled = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let blocked_by_conflict = state
                .incident
                .as_ref()
                .map(|incident| incident.kind == "conflict" || incident.kind == "push_conflict")
                .unwrap_or(false);
            if blocked_by_conflict || state.last_error.is_some() {
                false
            } else {
                state
                    .connection
                    .as_ref()
                    .map(SyncConnection::push_on_change)
                    .unwrap_or(false)
            }
        };
        if !enabled {
            return Ok(self.noop("Cloud sync auto-push is disabled"));
        }

        let _operation = self.operation_lock.lock().await;
        let current_hash = self.current_sync_hash()?;
        let already_synced = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.last_synced_hash.as_deref() == Some(current_hash.as_str())
        };
        if already_synced {
            return Ok(self.noop("Cloud sync is already up to date"));
        }

        match self.push_current_document(None, None).await {
            Ok(outcome) => {
                self.persist_operation_state(None, &outcome, None)?;
                Ok(self.operation_result(&outcome))
            }
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

    pub fn open_backup_file(&self, file_name: &str) -> Result<()> {
        ensure_sync_dirs()?;
        open_path(&backup_file_path(file_name)?)
    }

    pub fn list_backups(&self) -> Result<Vec<SyncBackupEntry>> {
        ensure_sync_dirs()?;
        Ok(list_backup_entries(
            Some(&crate::paths::sync_backups_dir()?),
        ))
    }

    pub fn preview_backup(&self, file_name: &str) -> Result<SyncBackupPreview> {
        ensure_sync_dirs()?;
        let path = backup_file_path(file_name)?;
        let metadata = std::fs::metadata(&path)
            .with_context(|| format!("Backup file not found: {file_name}"))?;
        if metadata.len() > 1024 * 1024 {
            anyhow::bail!("Backup file is too large to preview");
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read backup file: {file_name}"))?;
        Ok(SyncBackupPreview {
            file_name: file_name.to_string(),
            content,
        })
    }

    fn noop(&self, message: &str) -> SyncActionResult {
        SyncActionResult {
            message: message.to_string(),
            applied_remote: false,
            status: self.status(),
        }
    }

    async fn prepare_github_connect(
        &self,
        gist_id: String,
        pull_on_launch: bool,
        push_on_change: bool,
    ) -> Result<PreparedSyncConnectRequest> {
        let token =
            crate::credentials::github_bearer_token().context("GitHub account is not connected")?;
        if let Err(error) = validate_github_token(&token).await {
            anyhow::bail!(error.to_string());
        }
        let gist_id = if gist_id.trim().is_empty() {
            ensure_profile_gist(&self.client, &token)
                .await
                .map_err(|error| anyhow!(error.to_string()))?
        } else {
            gist_id.trim().to_string()
        };
        Ok(PreparedSyncConnectRequest {
            connection: SyncConnection::Github(super::types::GitHubSyncConnection {
                gist_id,
                pull_on_launch,
                push_on_change,
            }),
            github_token: Some(token),
        })
    }

    fn prepare_folder_connect(
        &self,
        folder_path: String,
        path: String,
        pull_on_launch: bool,
        push_on_change: bool,
    ) -> Result<PreparedSyncConnectRequest> {
        Ok(PreparedSyncConnectRequest {
            connection: SyncConnection::Folder(super::types::LocalFolderSyncConnection {
                folder_path: normalize_folder_path(&folder_path)?,
                path: normalize_path(&path)?,
                pull_on_launch,
                push_on_change,
            }),
            github_token: None,
        })
    }

    async fn pull(&self, mode: PullMode) -> Result<SyncActionResult> {
        let _operation = self.operation_lock.lock().await;
        let connection = self.current_connection()?;
        let remote = connection.fetch_remote_document(&self.client, None).await?;
        let Some(remote) = remote else {
            return self.fail_error("Remote profile file was not found");
        };
        let outcome = self.apply_remote_document(remote, mode).await?;
        self.persist_operation_state(None, &outcome, None)?;
        Ok(self.operation_result(&outcome))
    }

    async fn apply_remote_document(
        &self,
        remote: RemoteDocument,
        mode: PullMode,
    ) -> Result<SyncOperationOutcome> {
        let local_json = self.current_sync_document_json()?;
        let local_hash = hash_text(&local_json);
        let remote_hash = hash_text(&remote.content);
        let last_synced = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.last_synced_hash.clone()
        };

        let action = match mode {
            PullMode::Connect if local_hash != remote_hash => SyncAction::FastForwardFromRemote,
            _ => resolve_sync_action(&local_hash, &remote_hash, last_synced.as_deref()),
        };

        match action {
            SyncAction::NoOp => Ok(SyncOperationOutcome {
                message: pull_noop_message(mode),
                applied_remote: false,
                remote_revision: remote.revision,
                last_synced_hash_update: Some(remote_hash),
                incident: None,
            }),
            SyncAction::PushLocal => Ok(SyncOperationOutcome {
                message: pull_local_ahead_message(mode),
                applied_remote: false,
                remote_revision: remote.revision,
                last_synced_hash_update: None,
                incident: None,
            }),
            SyncAction::Conflict => {
                let backup_file = self.write_backup_file("conflict")?;
                let message = pull_conflict_message(mode);
                Ok(SyncOperationOutcome {
                    message: message.clone(),
                    applied_remote: false,
                    remote_revision: remote.revision,
                    last_synced_hash_update: None,
                    incident: Some(super::types::SyncIncident {
                        kind: "conflict".to_string(),
                        message,
                        backup_file: Some(filename_string(&backup_file)),
                        created_at: now_rfc3339(),
                    }),
                })
            }
            SyncAction::FastForwardFromRemote => {
                let backup_required = matches!(mode, PullMode::Connect);
                let backup_file = if backup_required {
                    Some(self.write_backup_file("remote-applied")?)
                } else {
                    None
                };
                let bundle: crate::features::profile::core::ProfileImportBundle =
                    serde_json::from_str(&remote.content)
                        .context("Remote profile JSON is invalid")?;
                let result =
                    crate::features::profile::core::apply_import_bundle(&self.plugins_dir, &bundle)
                        .await?;
                let message = pull_success_message(mode, backup_required, result.success);
                let incident = if backup_required || !result.success {
                    Some(super::types::SyncIncident {
                        kind: incident_kind(mode).to_string(),
                        message: message.clone(),
                        backup_file: backup_file.as_ref().map(|path| filename_string(path)),
                        created_at: now_rfc3339(),
                    })
                } else {
                    None
                };
                Ok(SyncOperationOutcome {
                    message,
                    applied_remote: true,
                    remote_revision: remote.revision,
                    last_synced_hash_update: Some(remote_hash),
                    incident,
                })
            }
        }
    }

    async fn push_current_document(
        &self,
        override_connection: Option<SyncConnection>,
        github_token: Option<&str>,
    ) -> Result<SyncOperationOutcome> {
        let uses_current_connection = override_connection.is_none();
        let connection = match override_connection {
            Some(connection) => connection,
            None => self.current_connection()?,
        };
        let local_json = self.current_sync_document_json()?;
        let local_hash = hash_text(&local_json);
        let remote = connection
            .fetch_remote_document(&self.client, github_token)
            .await?;
        if let Some(remote) = &remote {
            let remote_hash = hash_text(&remote.content);
            if remote_hash == local_hash {
                return Ok(SyncOperationOutcome {
                    message: "Cloud sync is already up to date".to_string(),
                    applied_remote: false,
                    remote_revision: remote.revision.clone(),
                    last_synced_hash_update: Some(local_hash),
                    incident: None,
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
        if should_resolve_push_conflict(
            uses_current_connection,
            stored_revision.as_deref(),
            remote_revision.as_deref(),
        ) {
            let remote = remote.context("Remote profile changed before push")?;
            return self.resolve_push_conflict(remote).await;
        }

        let next_revision = match connection
            .push_remote_document(
                &self.client,
                &local_json,
                remote_revision.as_deref(),
                github_token,
            )
            .await
        {
            Ok(revision) => revision,
            Err(ProviderError::Conflict(_)) => {
                let remote = connection
                    .fetch_remote_document(&self.client, github_token)
                    .await
                    .map_err(|error| anyhow!(error))?
                    .context("Remote profile changed before push")?;
                return self.resolve_push_conflict(remote).await;
            }
            Err(error) => return Err(anyhow!(error)),
        };

        Ok(SyncOperationOutcome {
            message: "Cloud sync pushed local changes".to_string(),
            applied_remote: false,
            remote_revision: next_revision,
            last_synced_hash_update: Some(local_hash),
            incident: None,
        })
    }

    async fn resolve_push_conflict(&self, remote: RemoteDocument) -> Result<SyncOperationOutcome> {
        let backup_file = self.write_backup_file("push-conflict")?;
        let bundle: crate::features::profile::core::ProfileImportBundle =
            serde_json::from_str(&remote.content).context("Remote profile JSON is invalid")?;
        let result =
            crate::features::profile::core::apply_import_bundle(&self.plugins_dir, &bundle).await?;
        let remote_hash = hash_text(&remote.content);
        let message = if result.success {
            "Remote profile changed first. Local profile was backed up and remote changes were applied."
        } else {
            "Remote profile changed first. Local profile was backed up and remote changes were applied with warnings."
        };

        Ok(SyncOperationOutcome {
            message: message.to_string(),
            applied_remote: true,
            remote_revision: remote.revision,
            last_synced_hash_update: Some(remote_hash),
            incident: Some(super::types::SyncIncident {
                kind: "push_conflict".to_string(),
                message: message.to_string(),
                backup_file: Some(filename_string(&backup_file)),
                created_at: now_rfc3339(),
            }),
        })
    }

    fn finalize_connect(
        &self,
        prepared: PreparedSyncConnectRequest,
        outcome: SyncOperationOutcome,
    ) -> Result<SyncActionResult> {
        self.persist_operation_state(Some(prepared.connection), &outcome, None)?;
        Ok(self.operation_result(&outcome))
    }

    fn persist_operation_state(
        &self,
        connection: Option<SyncConnection>,
        outcome: &SyncOperationOutcome,
        last_error: Option<&str>,
    ) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(connection) = connection {
            state.connection = Some(connection);
        }
        state.remote_revision = Some(outcome.remote_revision.clone());
        if let Some(hash) = &outcome.last_synced_hash_update {
            state.last_synced_hash = Some(hash.clone());
        }
        state.last_sync_at = Some(now_rfc3339());
        state.incident = outcome.incident.clone();
        state.last_error = last_error.map(str::to_string);
        save_state_file(&state)
    }

    fn operation_result(&self, outcome: &SyncOperationOutcome) -> SyncActionResult {
        SyncActionResult {
            message: outcome.message.clone(),
            applied_remote: outcome.applied_remote,
            status: self.status(),
        }
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
        let plugins = crate::features::profile::core::load_plugins_lock()
            .map(|lock| lock.plugins)
            .unwrap_or_default();
        crate::features::profile::core::build_sync_document_json(plugins)
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
        let plugins = crate::features::profile::core::load_plugins_lock()
            .map(|lock| lock.plugins)
            .unwrap_or_default();
        let content =
            crate::features::profile::core::build_export_bundle_json(now_rfc3339(), plugins)?;
        crate::file_io::atomic_write(&path, content.as_bytes())?;
        Ok(path)
    }

    fn return_error<T>(&self, error: anyhow::Error) -> Result<T> {
        log::error!("Cloud sync failed: {error:#}");
        if let Err(write_error) = self.store_last_error(&error.to_string()) {
            log::error!("Failed to persist cloud sync error state: {write_error:#}");
        }
        Err(error)
    }

    fn return_unpersisted_error<T>(&self, error: anyhow::Error) -> Result<T> {
        log::error!("Cloud sync failed: {error:#}");
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

fn should_resolve_push_conflict(
    uses_current_connection: bool,
    stored_revision: Option<&str>,
    remote_revision: Option<&str>,
) -> bool {
    if !uses_current_connection {
        return false;
    }
    if stored_revision.is_none() {
        return false;
    }
    if remote_revision.is_none() {
        return false;
    }
    remote_revision != stored_revision
}

#[cfg(test)]
mod tests {
    use super::should_resolve_push_conflict;

    #[test]
    fn push_conflict_check_only_applies_to_current_connection() {
        let cases = [
            (false, Some("old"), None, false),
            (false, Some("old"), Some("new"), false),
            (true, None, None, false),
            (true, None, Some("new"), false),
            (true, Some("same"), Some("same"), false),
            (true, Some("old"), Some("new"), true),
            (true, Some("old"), None, false),
        ];

        for (uses_current_connection, stored_revision, remote_revision, expected) in cases {
            assert_eq!(
                should_resolve_push_conflict(
                    uses_current_connection,
                    stored_revision,
                    remote_revision
                ),
                expected
            );
        }
    }
}
