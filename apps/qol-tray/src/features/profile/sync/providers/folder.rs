use anyhow::Result;
use std::path::PathBuf;

use super::{ProviderError, RemoteDocument};
use crate::features::profile::sync::LocalFolderSyncConnection;

pub(super) fn validate_connection(connection: &LocalFolderSyncConnection) -> Result<()> {
    normalize_folder_path(&connection.folder_path)?;
    if !super::is_safe_remote_path(&connection.path) {
        anyhow::bail!("Invalid remote path");
    }
    Ok(())
}

pub(super) fn normalize_folder_path(folder_path: &str) -> Result<String> {
    let trimmed = folder_path.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Folder path cannot be empty");
    }
    let path = PathBuf::from(trimmed);
    if !path.is_absolute() {
        anyhow::bail!("Folder path must be absolute");
    }
    Ok(path.display().to_string())
}

pub(super) fn folder_sync_target_path(connection: &LocalFolderSyncConnection) -> PathBuf {
    PathBuf::from(&connection.folder_path).join(&connection.path)
}

pub(super) fn fetch_remote_document(
    connection: &LocalFolderSyncConnection,
) -> std::result::Result<Option<RemoteDocument>, ProviderError> {
    let path = folder_sync_target_path(connection);
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|error| ProviderError::Upstream(format!("Failed to read sync file: {}", error)))?;
    Ok(Some(RemoteDocument {
        revision: crate::features::profile::sync::state::hash_text(&content),
        content,
    }))
}

pub(super) fn push_remote_document(
    connection: &LocalFolderSyncConnection,
    content: &str,
    remote_revision: Option<&str>,
) -> std::result::Result<String, ProviderError> {
    let path = folder_sync_target_path(connection);
    if let Some(remote_revision) = remote_revision {
        if !path.exists() {
            return Err(ProviderError::Conflict(
                "Folder sync file changed before push".to_string(),
            ));
        }
        let current = std::fs::read_to_string(&path).map_err(|error| {
            ProviderError::Upstream(format!("Failed to read sync file: {}", error))
        })?;
        if crate::features::profile::sync::state::hash_text(&current) != remote_revision {
            return Err(ProviderError::Conflict(
                "Folder sync file changed before push".to_string(),
            ));
        }
    }
    crate::file_io::ensure_parent_dir(&path).map_err(|error| {
        ProviderError::Upstream(format!("Failed to prepare sync folder: {}", error))
    })?;
    crate::file_io::atomic_write(&path, content.as_bytes()).map_err(|error| {
        ProviderError::Upstream(format!("Failed to write sync file: {}", error))
    })?;
    Ok(crate::features::profile::sync::state::hash_text(content))
}
