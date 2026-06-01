use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;

use super::operation_lock::{self, PluginOperationLock};
use super::staging;

#[derive(Debug)]
pub struct OperationLockHandle {
    _inner: PluginOperationLock,
}

pub fn acquire_operation_lock(plugins_dir: &Path, plugin_id: &str) -> Result<OperationLockHandle> {
    let inner = operation_lock::acquire_operation_lock(plugins_dir, plugin_id)?;
    Ok(OperationLockHandle { _inner: inner })
}

pub fn lockfile_path(plugins_dir: &Path, plugin_id: &str) -> PathBuf {
    operation_lock::lock_path(plugins_dir, plugin_id)
}

pub fn lockfile_max_age() -> Duration {
    super::LOCKFILE_MAX_AGE
}

pub fn install_staging_dir(plugins_dir: &Path, plugin_id: &str) -> PathBuf {
    staging::install_staging_dir(plugins_dir, plugin_id)
}

pub fn update_staging_dir(plugins_dir: &Path, plugin_id: &str) -> PathBuf {
    staging::update_staging_dir(plugins_dir, plugin_id)
}

pub fn update_backup_dir(plugins_dir: &Path, plugin_id: &str) -> PathBuf {
    staging::update_backup_dir(plugins_dir, plugin_id)
}

pub async fn swap_plugin_dirs(
    live_dir: &Path,
    staging_dir: &Path,
    backup_dir: &Path,
) -> Result<()> {
    staging::swap_plugin_dirs(live_dir, staging_dir, backup_dir).await
}
