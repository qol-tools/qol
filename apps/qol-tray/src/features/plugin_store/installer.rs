use anyhow::{Context, Result};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod command;
mod dependency;
mod lock;
mod source;
mod staging;

use dependency::install_dependencies;
use lock::{open_lock_file, stale_lockfile};
use source::{clone_plugin_repo, prepare_update_repo};
use staging::{
    cleanup_temp_dir, install_staging_dir, swap_plugin_dirs, update_backup_dir, update_staging_dir,
};

const GIT_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_CARGO_BUILD_JOBS: usize = 4;
const CARGO_BUILD_TIMEOUT: Duration = Duration::from_secs(300);
#[cfg(unix)]
const LOCKFILE_MAX_AGE: Duration = Duration::from_secs(30);
#[cfg(not(unix))]
const LOCKFILE_MAX_AGE: Duration = Duration::from_secs(300);

pub struct PluginInstaller {
    plugins_dir: PathBuf,
}

impl PluginInstaller {
    pub fn new(plugins_dir: PathBuf) -> Self {
        Self { plugins_dir }
    }

    pub async fn install(&self, repo_url: &str, plugin_id: &str) -> Result<()> {
        validate_plugin_id(plugin_id)?;
        let _operation_lock = self.acquire_operation_lock(plugin_id)?;
        Self::check_dev_link_conflict(plugin_id)?;

        let target_dir = self.plugins_dir.join(plugin_id);
        ensure_not_installed(&target_dir, plugin_id)?;

        let staging_dir = install_staging_dir(&self.plugins_dir, plugin_id);
        let plan = InstallPlan {
            repo_url,
            plugin_id,
            staging_dir: &staging_dir,
            target_dir: &target_dir,
        };
        let result = self.install_plugin(plan).await;
        finish_with_cleanup(&staging_dir, result).await
    }

    async fn install_plugin(&self, plan: InstallPlan<'_>) -> Result<()> {
        clone_plugin_repo(plan.repo_url, plan.staging_dir).await?;
        install_dependencies(plan.plugin_id, plan.staging_dir).await?;
        finalize_install(plan.staging_dir, plan.target_dir).await?;
        log::info!("Plugin {} installed successfully", plan.plugin_id);
        Ok(())
    }

    pub async fn update(&self, repo_url: &str, plugin_id: &str) -> Result<()> {
        validate_plugin_id(plugin_id)?;
        let _operation_lock = self.acquire_operation_lock(plugin_id)?;
        Self::check_dev_link_conflict(plugin_id)?;

        let plugin_dir = self.plugins_dir.join(plugin_id);
        ensure_installed(&plugin_dir, plugin_id)?;

        let staging_dir = update_staging_dir(&self.plugins_dir, plugin_id);
        let backup_dir = update_backup_dir(&self.plugins_dir, plugin_id);
        let plan = UpdatePlan {
            repo_url,
            plugin_id,
            plugin_dir: &plugin_dir,
            staging_dir: &staging_dir,
            backup_dir: &backup_dir,
        };
        let result = self.update_plugin(plan).await;
        finish_with_cleanup(&staging_dir, result).await
    }

    async fn update_plugin(&self, plan: UpdatePlan<'_>) -> Result<()> {
        log::info!("Updating plugin {} from {}", plan.plugin_id, plan.repo_url);
        prepare_update_repo(plan.staging_dir, plan.repo_url).await?;
        install_dependencies(plan.plugin_id, plan.staging_dir).await?;
        swap_plugin_dirs(plan.plugin_dir, plan.staging_dir, plan.backup_dir).await?;
        log::info!("Plugin {} updated successfully", plan.plugin_id);
        Ok(())
    }

    pub async fn uninstall(&self, plugin_id: &str) -> Result<()> {
        validate_plugin_id(plugin_id)?;
        let _operation_lock = self.acquire_operation_lock(plugin_id)?;

        let plugin_dir = self.plugins_dir.join(plugin_id);
        ensure_installed(&plugin_dir, plugin_id)?;

        log::info!("Uninstalling plugin: {}", plugin_id);
        tokio::fs::remove_dir_all(&plugin_dir).await?;
        log::info!("Plugin {} uninstalled successfully", plugin_id);
        Ok(())
    }

    fn acquire_operation_lock(&self, plugin_id: &str) -> Result<PluginOperationLock> {
        ensure_plugins_dir(&self.plugins_dir)?;
        let path = lock_path(&self.plugins_dir, plugin_id);

        match open_lock_file(&path) {
            Ok(mut file) => create_operation_lock(&path, plugin_id, &mut file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                reacquire_stale_lock(&path, plugin_id)
            }
            Err(error) => Err(error).with_context(|| {
                format!("Failed to acquire plugin operation lock {}", path.display())
            }),
        }
    }

    #[cfg(feature = "dev")]
    fn check_dev_link_conflict(plugin_id: &str) -> Result<()> {
        let config_dir = crate::paths::shared_config_dir()?;
        let dev_links = crate::dev::load_dev_links(&config_dir);
        if dev_links.contains_key(plugin_id) {
            anyhow::bail!(
                "Cannot proceed — {} is dev-linked. Unlink first.",
                plugin_id
            );
        }
        Ok(())
    }

    #[cfg(not(feature = "dev"))]
    fn check_dev_link_conflict(_plugin_id: &str) -> Result<()> {
        Ok(())
    }
}

struct InstallPlan<'a> {
    repo_url: &'a str,
    plugin_id: &'a str,
    staging_dir: &'a Path,
    target_dir: &'a Path,
}

struct UpdatePlan<'a> {
    repo_url: &'a str,
    plugin_id: &'a str,
    plugin_dir: &'a Path,
    staging_dir: &'a Path,
    backup_dir: &'a Path,
}

async fn finish_with_cleanup(staging_dir: &Path, result: Result<()>) -> Result<()> {
    if result.is_err() {
        cleanup_temp_dir(staging_dir).await;
    }
    result
}

async fn finalize_install(staging_dir: &Path, target_dir: &Path) -> Result<()> {
    tokio::fs::rename(staging_dir, target_dir)
        .await
        .with_context(|| {
            format!(
                "Failed to finalize plugin install from {:?} to {:?}",
                staging_dir, target_dir
            )
        })
}

fn ensure_not_installed(plugin_dir: &Path, plugin_id: &str) -> Result<()> {
    if !plugin_dir.exists() {
        return Ok(());
    }
    anyhow::bail!("Plugin already installed: {}", plugin_id)
}

fn ensure_installed(plugin_dir: &Path, plugin_id: &str) -> Result<()> {
    if plugin_dir.exists() {
        return Ok(());
    }
    anyhow::bail!("Plugin not installed: {}", plugin_id)
}

fn ensure_plugins_dir(plugins_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(plugins_dir).with_context(|| {
        format!(
            "Failed to create plugins directory {}",
            plugins_dir.display()
        )
    })
}

fn lock_path(plugins_dir: &Path, plugin_id: &str) -> PathBuf {
    plugins_dir.join(format!(".{}.lock", plugin_id))
}

fn create_operation_lock(
    path: &Path,
    plugin_id: &str,
    file: &mut std::fs::File,
) -> Result<PluginOperationLock> {
    write_lock_owner(file, plugin_id);
    Ok(PluginOperationLock {
        path: path.to_path_buf(),
    })
}

fn write_lock_owner(file: &mut std::fs::File, plugin_id: &str) {
    let _ = writeln!(file, "{} {}", std::process::id(), plugin_id);
}

fn reacquire_stale_lock(path: &Path, plugin_id: &str) -> Result<PluginOperationLock> {
    if !stale_lockfile(path, LOCKFILE_MAX_AGE) {
        anyhow::bail!("Plugin operation already in progress: {}", plugin_id);
    }

    let _ = std::fs::remove_file(path);
    let mut file = open_lock_file(path).with_context(|| {
        format!(
            "Failed to reacquire stale plugin operation lock {}",
            path.display()
        )
    })?;
    create_operation_lock(path, plugin_id, &mut file)
}

struct PluginOperationLock {
    path: PathBuf,
}

impl Drop for PluginOperationLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn validate_plugin_id(plugin_id: &str) -> Result<()> {
    if super::validation::is_safe_plugin_id(plugin_id) {
        return Ok(());
    }
    anyhow::bail!("{}: {}", super::validation::INVALID_PLUGIN_ID, plugin_id)
}
