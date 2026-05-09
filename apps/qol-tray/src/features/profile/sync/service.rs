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
    SyncIncidentKind, SyncProviderDefinition,
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
        let decision = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            launch_pull_decision(&state)
        };
        match decision {
            LaunchPullDecision::Skip(message) => Ok(self.noop(message)),
            LaunchPullDecision::Proceed => match self.pull(PullMode::Launch).await {
                Ok(result) => Ok(result),
                Err(error) => self.return_error(error),
            },
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
                .is_some_and(|incident| incident.kind.blocks_auto_sync());
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
                        kind: SyncIncidentKind::Conflict,
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
                        kind: incident_kind(mode),
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
                kind: SyncIncidentKind::PushConflict,
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

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LaunchPullDecision {
    Skip(&'static str),
    Proceed,
}

pub(crate) fn launch_pull_decision(state: &SyncStateFile) -> LaunchPullDecision {
    if state
        .incident
        .as_ref()
        .is_some_and(|incident| incident.kind.blocks_auto_sync())
    {
        return LaunchPullDecision::Skip("Cloud sync pull skipped: unresolved conflict");
    }
    let pull_on_launch_enabled = state
        .connection
        .as_ref()
        .map(SyncConnection::pull_on_launch)
        .unwrap_or(false);
    if !pull_on_launch_enabled {
        return LaunchPullDecision::Skip("Cloud sync launch pull is disabled");
    }
    LaunchPullDecision::Proceed
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
    use super::*;
    use crate::features::profile::sync::types::{
        GitHubSyncConnection, LocalFolderSyncConnection, SyncIncident,
    };

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

    fn github_connection(pull_on_launch: bool) -> SyncConnection {
        SyncConnection::Github(GitHubSyncConnection {
            gist_id: "abc123".to_string(),
            pull_on_launch,
            push_on_change: true,
        })
    }

    fn incident_with_kind(kind: SyncIncidentKind) -> SyncIncident {
        SyncIncident {
            kind,
            message: String::new(),
            backup_file: None,
            created_at: now_rfc3339(),
        }
    }

    #[test]
    fn launch_pull_decision_skips_without_connection() {
        let state = SyncStateFile::default();
        assert_eq!(
            launch_pull_decision(&state),
            LaunchPullDecision::Skip("Cloud sync launch pull is disabled"),
        );
    }

    #[test]
    fn launch_pull_decision_skips_when_pull_on_launch_disabled() {
        let state = SyncStateFile {
            connection: Some(github_connection(false)),
            ..SyncStateFile::default()
        };
        assert_eq!(
            launch_pull_decision(&state),
            LaunchPullDecision::Skip("Cloud sync launch pull is disabled"),
        );
    }

    #[test]
    fn launch_pull_decision_proceeds_when_connected_and_no_incident() {
        let state = SyncStateFile {
            connection: Some(github_connection(true)),
            ..SyncStateFile::default()
        };
        assert_eq!(launch_pull_decision(&state), LaunchPullDecision::Proceed);
    }

    #[test]
    fn launch_pull_decision_blocks_only_on_conflict_kinds() {
        let cases = [
            (
                SyncIncidentKind::Conflict,
                LaunchPullDecision::Skip(CONFLICT_SKIP),
            ),
            (
                SyncIncidentKind::PushConflict,
                LaunchPullDecision::Skip(CONFLICT_SKIP),
            ),
            (
                SyncIncidentKind::LaunchPullReview,
                LaunchPullDecision::Proceed,
            ),
            (
                SyncIncidentKind::ManualPullReview,
                LaunchPullDecision::Proceed,
            ),
            (
                SyncIncidentKind::ConnectPullReview,
                LaunchPullDecision::Proceed,
            ),
            (SyncIncidentKind::Unknown, LaunchPullDecision::Proceed),
        ];
        for (kind, expected) in cases {
            let state = SyncStateFile {
                connection: Some(github_connection(true)),
                incident: Some(incident_with_kind(kind)),
                ..SyncStateFile::default()
            };
            assert_eq!(launch_pull_decision(&state), expected, "kind: {kind:?}");
        }
    }

    #[test]
    fn launch_pull_decision_blocks_even_when_pull_on_launch_disabled() {
        let state = SyncStateFile {
            connection: Some(github_connection(false)),
            incident: Some(incident_with_kind(SyncIncidentKind::Conflict)),
            ..SyncStateFile::default()
        };
        assert_eq!(
            launch_pull_decision(&state),
            LaunchPullDecision::Skip(CONFLICT_SKIP),
            "conflict gate runs before the pull_on_launch flag check",
        );
    }

    #[test]
    fn launch_pull_decision_ignores_last_error() {
        let state = SyncStateFile {
            connection: Some(github_connection(true)),
            last_error: Some("network unreachable".to_string()),
            ..SyncStateFile::default()
        };
        assert_eq!(
            launch_pull_decision(&state),
            LaunchPullDecision::Proceed,
            "last_error gates auto_push_if_dirty but not pull_on_launch",
        );
    }

    const CONFLICT_SKIP: &str = "Cloud sync pull skipped: unresolved conflict";

    struct SyncTestEnv {
        _config_root: tempfile::TempDir,
        remote_dir: tempfile::TempDir,
        _path_guard: crate::paths::TestPathRootGuard,
    }

    impl SyncTestEnv {
        fn new() -> Self {
            let _config_root = tempfile::TempDir::new().unwrap();
            let _path_guard = crate::paths::push_test_path_root(_config_root.path());
            let remote_dir = tempfile::TempDir::new().unwrap();
            std::fs::create_dir_all(crate::paths::plugins_dir().unwrap()).unwrap();
            Self {
                _config_root,
                remote_dir,
                _path_guard,
            }
        }

        fn plugins_dir(&self) -> std::path::PathBuf {
            crate::paths::plugins_dir().unwrap()
        }

        fn folder_request(&self, pull_on_launch: bool, push_on_change: bool) -> SyncConnectRequest {
            SyncConnectRequest::Folder {
                folder_path: self.remote_dir.path().display().to_string(),
                path: "profile.json".to_string(),
                pull_on_launch,
                push_on_change,
            }
        }

        fn folder_connection(&self, pull_on_launch: bool) -> SyncConnection {
            SyncConnection::Folder(LocalFolderSyncConnection {
                folder_path: self.remote_dir.path().display().to_string(),
                path: "profile.json".to_string(),
                pull_on_launch,
                push_on_change: true,
            })
        }

        fn write_remote_bundle(&self, bundle: serde_json::Value) {
            std::fs::write(
                self.remote_dir.path().join("profile.json"),
                bundle.to_string(),
            )
            .unwrap();
        }

        fn write_local_hotkeys(&self, hotkeys: serde_json::Value) {
            let path = crate::paths::hotkeys_path().unwrap();
            crate::file_io::ensure_parent_dir(&path).unwrap();
            std::fs::write(path, serde_json::json!({ "hotkeys": hotkeys }).to_string()).unwrap();
        }

        fn read_local_hotkeys(&self) -> serde_json::Value {
            let path = crate::paths::hotkeys_path().unwrap();
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            serde_json::from_str::<serde_json::Value>(&content)
                .ok()
                .and_then(|v| v.get("hotkeys").cloned())
                .unwrap_or(serde_json::Value::Null)
        }

        fn list_backups(&self) -> Vec<String> {
            let dir = match crate::paths::sync_backups_dir() {
                Ok(dir) => dir,
                Err(_) => return Vec::new(),
            };
            std::fs::read_dir(&dir)
                .map(|entries| {
                    entries
                        .filter_map(|entry| entry.ok())
                        .filter_map(|entry| entry.file_name().into_string().ok())
                        .collect()
                })
                .unwrap_or_default()
        }

        fn load_persisted_state(&self) -> SyncStateFile {
            super::load_state_file().unwrap()
        }
    }

    #[tokio::test]
    async fn connect_with_diverging_remote_writes_backup_and_applies_remote() {
        let env = SyncTestEnv::new();
        env.write_local_hotkeys(serde_json::json!([{ "id": "h1" }, { "id": "h2" }]));
        env.write_remote_bundle(serde_json::json!({ "hotkeys": [] }));

        let service = SyncService::new(env.plugins_dir()).unwrap();
        let result = service
            .connect(env.folder_request(true, true))
            .await
            .unwrap();

        assert!(result.applied_remote, "Connect must apply diverging remote");
        assert_eq!(
            env.read_local_hotkeys(),
            serde_json::json!([]),
            "local hotkeys overwritten by empty remote (this is the user-reported wipe vector)",
        );
        let backups = env.list_backups();
        assert_eq!(backups.len(), 1, "exactly one backup, got: {backups:?}");
        assert!(
            backups[0].contains("remote-applied"),
            "backup name should include 'remote-applied' tag, got: {}",
            backups[0],
        );
        let state = env.load_persisted_state();
        assert_eq!(
            state.incident.as_ref().map(|i| i.kind),
            Some(SyncIncidentKind::ConnectPullReview),
            "Connect mode incident is review, NOT Conflict - so the launch-pull conflict gate does NOT block subsequent pulls in this scenario",
        );
    }

    #[tokio::test]
    async fn pull_returns_conflict_when_local_and_remote_diverge_from_last_synced() {
        let env = SyncTestEnv::new();
        env.write_local_hotkeys(serde_json::json!([{ "id": "shared" }]));
        env.write_remote_bundle(serde_json::json!({ "hotkeys": [{ "id": "shared" }] }));

        let service = SyncService::new(env.plugins_dir()).unwrap();
        service
            .connect(env.folder_request(true, true))
            .await
            .unwrap();

        env.write_local_hotkeys(serde_json::json!([{ "id": "local-only" }]));
        env.write_remote_bundle(serde_json::json!({ "hotkeys": [{ "id": "remote-only" }] }));

        let result = service.manual_pull().await.unwrap();

        assert!(
            !result.applied_remote,
            "conflict path must not apply remote"
        );
        assert_eq!(
            env.read_local_hotkeys(),
            serde_json::json!([{ "id": "local-only" }]),
            "local must be preserved through a conflict",
        );
        let state = env.load_persisted_state();
        assert_eq!(
            state.incident.as_ref().map(|i| i.kind),
            Some(SyncIncidentKind::Conflict),
            "incident kind must be Conflict so the launch-pull gate fires next restart",
        );
        assert!(
            env.list_backups()
                .iter()
                .any(|name| name.contains("conflict")),
            "conflict path must write a backup of local-at-divergence",
        );
    }

    #[tokio::test]
    async fn pull_on_launch_is_gated_when_persisted_state_has_conflict_incident() {
        let env = SyncTestEnv::new();
        env.write_local_hotkeys(serde_json::json!([{ "id": "preserved" }]));
        env.write_remote_bundle(serde_json::json!({ "hotkeys": [{ "id": "tempting-but-stale" }] }));

        let pre_state = SyncStateFile {
            connection: Some(env.folder_connection(true)),
            last_synced_hash: Some("any-stale-hash".to_string()),
            incident: Some(SyncIncident {
                kind: SyncIncidentKind::Conflict,
                message: "diverged".to_string(),
                backup_file: None,
                created_at: now_rfc3339(),
            }),
            ..SyncStateFile::default()
        };
        save_state_file(&pre_state).unwrap();

        let service = SyncService::new(env.plugins_dir()).unwrap();
        let result = service.pull_on_launch().await.unwrap();

        assert_eq!(result.message, CONFLICT_SKIP);
        assert!(!result.applied_remote);
        assert_eq!(
            env.read_local_hotkeys(),
            serde_json::json!([{ "id": "preserved" }]),
            "gate must keep local intact across restart",
        );
    }

    #[tokio::test]
    async fn acknowledge_incident_clears_only_the_incident_field() {
        let env = SyncTestEnv::new();
        let pre_state = SyncStateFile {
            connection: Some(env.folder_connection(true)),
            last_synced_hash: Some("preserved-hash".to_string()),
            remote_revision: Some("preserved-rev".to_string()),
            last_sync_at: Some("2026-05-01T00:00:00Z".to_string()),
            incident: Some(SyncIncident {
                kind: SyncIncidentKind::Conflict,
                message: "diverged".to_string(),
                backup_file: Some("20260501-000000-conflict.json".to_string()),
                created_at: now_rfc3339(),
            }),
            last_error: None,
        };
        save_state_file(&pre_state).unwrap();

        let service = SyncService::new(env.plugins_dir()).unwrap();
        service.acknowledge_incident().await.unwrap();

        let state = env.load_persisted_state();
        assert!(state.incident.is_none(), "incident must clear");
        assert_eq!(
            state.last_synced_hash.as_deref(),
            Some("preserved-hash"),
            "last_synced_hash is intentionally preserved by acknowledge",
        );
        assert_eq!(state.remote_revision.as_deref(), Some("preserved-rev"));
        assert_eq!(state.last_sync_at.as_deref(), Some("2026-05-01T00:00:00Z"));
        assert!(state.connection.is_some());
    }

    #[tokio::test]
    async fn disconnect_zeroes_every_state_field() {
        let env = SyncTestEnv::new();
        let pre_state = SyncStateFile {
            connection: Some(env.folder_connection(true)),
            last_synced_hash: Some("hash".to_string()),
            remote_revision: Some("rev".to_string()),
            last_sync_at: Some("2026-01-01T00:00:00Z".to_string()),
            incident: Some(SyncIncident {
                kind: SyncIncidentKind::Conflict,
                message: "test".to_string(),
                backup_file: None,
                created_at: now_rfc3339(),
            }),
            last_error: Some("err".to_string()),
        };
        save_state_file(&pre_state).unwrap();

        let service = SyncService::new(env.plugins_dir()).unwrap();
        service.disconnect().await.unwrap();

        let state = env.load_persisted_state();
        assert!(state.connection.is_none());
        assert!(state.last_synced_hash.is_none());
        assert!(state.remote_revision.is_none());
        assert!(state.last_sync_at.is_none());
        assert!(state.incident.is_none());
        assert!(state.last_error.is_none());
    }
}
