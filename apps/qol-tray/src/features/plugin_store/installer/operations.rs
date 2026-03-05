use super::dependency::install_dependencies;
use super::source::{clone_plugin_repo, prepare_update_repo};
use super::staging::{
    cleanup_temp_dir, install_staging_dir, swap_plugin_dirs, update_backup_dir, update_staging_dir,
};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub(super) async fn install(plugins_dir: &Path, repo_url: &str, plugin_id: &str) -> Result<()> {
    let plan = InstallPlan::new(plugins_dir, repo_url, plugin_id);
    ensure_not_installed(&plan.target_dir, plugin_id)?;
    let result = install_plugin(&plan).await;
    finish_with_cleanup(&plan.staging_dir, result).await
}

pub(super) async fn update(plugins_dir: &Path, repo_url: &str, plugin_id: &str) -> Result<()> {
    let plan = UpdatePlan::new(plugins_dir, repo_url, plugin_id);
    ensure_installed(&plan.plugin_dir, plugin_id)?;
    let result = update_plugin(&plan).await;
    finish_with_cleanup(&plan.staging_dir, result).await
}

pub(super) async fn uninstall(plugins_dir: &Path, plugin_id: &str) -> Result<()> {
    let plugin_dir = plugins_dir.join(plugin_id);
    ensure_installed(&plugin_dir, plugin_id)?;
    log::info!("Uninstalling plugin: {}", plugin_id);
    tokio::fs::remove_dir_all(&plugin_dir).await?;
    log::info!("Plugin {} uninstalled successfully", plugin_id);
    Ok(())
}

#[cfg(feature = "dev")]
pub(super) fn ensure_no_dev_link_conflict(plugin_id: &str) -> Result<()> {
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
pub(super) fn ensure_no_dev_link_conflict(_plugin_id: &str) -> Result<()> {
    Ok(())
}

struct InstallPlan<'a> {
    repo_url: &'a str,
    plugin_id: &'a str,
    staging_dir: PathBuf,
    target_dir: PathBuf,
}

impl<'a> InstallPlan<'a> {
    fn new(plugins_dir: &Path, repo_url: &'a str, plugin_id: &'a str) -> Self {
        Self {
            repo_url,
            plugin_id,
            staging_dir: install_staging_dir(plugins_dir, plugin_id),
            target_dir: plugins_dir.join(plugin_id),
        }
    }
}

struct UpdatePlan<'a> {
    repo_url: &'a str,
    plugin_id: &'a str,
    plugin_dir: PathBuf,
    staging_dir: PathBuf,
    backup_dir: PathBuf,
}

impl<'a> UpdatePlan<'a> {
    fn new(plugins_dir: &Path, repo_url: &'a str, plugin_id: &'a str) -> Self {
        Self {
            repo_url,
            plugin_id,
            plugin_dir: plugins_dir.join(plugin_id),
            staging_dir: update_staging_dir(plugins_dir, plugin_id),
            backup_dir: update_backup_dir(plugins_dir, plugin_id),
        }
    }
}

async fn install_plugin(plan: &InstallPlan<'_>) -> Result<()> {
    clone_plugin_repo(plan.repo_url, &plan.staging_dir).await?;
    install_dependencies(plan.plugin_id, &plan.staging_dir).await?;
    finalize_install(&plan.staging_dir, &plan.target_dir).await?;
    log::info!("Plugin {} installed successfully", plan.plugin_id);
    Ok(())
}

async fn update_plugin(plan: &UpdatePlan<'_>) -> Result<()> {
    log::info!("Updating plugin {} from {}", plan.plugin_id, plan.repo_url);
    prepare_update_repo(&plan.staging_dir, plan.repo_url).await?;
    install_dependencies(plan.plugin_id, &plan.staging_dir).await?;
    swap_plugin_dirs(&plan.plugin_dir, &plan.staging_dir, &plan.backup_dir).await?;
    log::info!("Plugin {} updated successfully", plan.plugin_id);
    Ok(())
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
