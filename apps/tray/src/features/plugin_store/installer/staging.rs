use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) fn install_staging_dir(plugins_dir: &Path, plugin_id: &str) -> PathBuf {
    temporary_plugin_dir(plugins_dir, plugin_id, "installing")
}

pub(super) fn update_staging_dir(plugins_dir: &Path, plugin_id: &str) -> PathBuf {
    temporary_plugin_dir(plugins_dir, plugin_id, "updating")
}

pub(super) fn update_backup_dir(plugins_dir: &Path, plugin_id: &str) -> PathBuf {
    temporary_plugin_dir(plugins_dir, plugin_id, "backup")
}

fn temporary_plugin_dir(plugins_dir: &Path, plugin_id: &str, phase: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    plugins_dir.join(format!(
        ".{}.{}.{}.{}",
        plugin_id,
        phase,
        std::process::id(),
        nanos
    ))
}

pub(super) async fn cleanup_temp_dir(path: &Path) {
    if tokio::fs::metadata(path).await.is_err() {
        return;
    }

    if let Err(error) = tokio::fs::remove_dir_all(path).await {
        log::warn!(
            "Failed to cleanup temporary plugin directory {:?}: {}",
            path,
            error
        );
    }
}

pub(super) async fn swap_plugin_dirs(
    live_dir: &Path,
    staging_dir: &Path,
    backup_dir: &Path,
) -> Result<()> {
    let plan = SwapPlan {
        live_dir,
        staging_dir,
        backup_dir,
    };

    backup_live_dir(&plan).await?;

    let result = tokio::fs::rename(plan.staging_dir, plan.live_dir).await;
    let Err(error) = result else {
        cleanup_backup_dir(&plan).await;
        return Ok(());
    };

    rollback_swap(&plan, error).await
}

struct SwapPlan<'a> {
    live_dir: &'a Path,
    staging_dir: &'a Path,
    backup_dir: &'a Path,
}

async fn backup_live_dir(plan: &SwapPlan<'_>) -> Result<()> {
    tokio::fs::rename(plan.live_dir, plan.backup_dir)
        .await
        .with_context(|| {
            format!(
                "Failed to move plugin directory {:?} to backup {:?}",
                plan.live_dir, plan.backup_dir
            )
        })
}

async fn cleanup_backup_dir(plan: &SwapPlan<'_>) {
    if let Err(error) = tokio::fs::remove_dir_all(plan.backup_dir).await {
        log::warn!(
            "Failed to cleanup plugin backup directory {:?}: {}",
            plan.backup_dir,
            error
        );
    }
}

async fn rollback_swap(plan: &SwapPlan<'_>, swap_error: std::io::Error) -> Result<()> {
    if let Err(rollback_error) = tokio::fs::rename(plan.backup_dir, plan.live_dir).await {
        anyhow::bail!(
            "Failed to swap plugin directories: {}; rollback failed: {}",
            swap_error,
            rollback_error
        );
    }

    anyhow::bail!("Failed to swap plugin directories: {}", swap_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn install_staging_dir_uses_hidden_plugin_scoped_prefix() {
        let root = TempDir::new().unwrap();
        let staging = install_staging_dir(root.path(), "plugin-test");
        let name = staging.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(".plugin-test.installing."));
    }

    #[test]
    fn update_staging_dir_uses_hidden_plugin_scoped_prefix() {
        let root = TempDir::new().unwrap();
        let staging = update_staging_dir(root.path(), "plugin-test");
        let name = staging.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(".plugin-test.updating."));
    }

    #[test]
    fn update_backup_dir_uses_hidden_plugin_scoped_prefix() {
        let root = TempDir::new().unwrap();
        let backup = update_backup_dir(root.path(), "plugin-test");
        let name = backup.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with(".plugin-test.backup."));
    }

    #[tokio::test]
    async fn cleanup_temp_dir_removes_directory_tree() {
        let root = TempDir::new().unwrap();
        let install_dir = root.path().join(".plugin-test.installing.1");

        tokio::fs::create_dir_all(install_dir.join("nested"))
            .await
            .unwrap();
        tokio::fs::write(install_dir.join("nested").join("file.txt"), b"x")
            .await
            .unwrap();

        cleanup_temp_dir(&install_dir).await;
        assert!(!install_dir.exists());
    }

    #[tokio::test]
    async fn swap_plugin_dirs_replaces_live_with_staging() {
        let root = TempDir::new().unwrap();
        let live = root.path().join("plugin-test");
        let staging = root.path().join(".plugin-test.updating.1");
        let backup = root.path().join(".plugin-test.backup.1");

        tokio::fs::create_dir_all(&live).await.unwrap();
        tokio::fs::create_dir_all(&staging).await.unwrap();
        tokio::fs::write(live.join("old.txt"), b"old")
            .await
            .unwrap();
        tokio::fs::write(staging.join("new.txt"), b"new")
            .await
            .unwrap();

        swap_plugin_dirs(&live, &staging, &backup).await.unwrap();

        assert!(tokio::fs::metadata(live.join("new.txt")).await.is_ok());
        assert!(tokio::fs::metadata(live.join("old.txt")).await.is_err());
        assert!(tokio::fs::metadata(&backup).await.is_err());
    }

    #[tokio::test]
    async fn swap_plugin_dirs_rolls_back_when_staging_missing() {
        let root = TempDir::new().unwrap();
        let live = root.path().join("plugin-test");
        let staging = root.path().join(".plugin-test.updating.1");
        let backup = root.path().join(".plugin-test.backup.1");

        tokio::fs::create_dir_all(&live).await.unwrap();
        tokio::fs::write(live.join("old.txt"), b"old")
            .await
            .unwrap();

        assert!(swap_plugin_dirs(&live, &staging, &backup).await.is_err());
        assert!(tokio::fs::metadata(live.join("old.txt")).await.is_ok());
        assert!(tokio::fs::metadata(&backup).await.is_err());
    }
}
