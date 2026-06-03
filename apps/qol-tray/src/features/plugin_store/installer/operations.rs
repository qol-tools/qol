use super::dependency::install_dependencies;
use super::source::{clone_plugin_repo, prepare_update_repo};
use super::staging::{
    cleanup_temp_dir, install_staging_dir, swap_plugin_dirs, update_backup_dir, update_staging_dir,
};
use super::InstallSource;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub(super) async fn install(
    plugins_dir: &Path,
    repo_url: &str,
    plugin_id: &str,
    install_source: InstallSource,
) -> Result<()> {
    let plan = InstallPlan::new(plugins_dir, repo_url, plugin_id, install_source);
    if active_is_live_source(plugin_id) {
        clear_stale_fallback(&plan.target_dir, plugin_id).await;
    } else {
        ensure_not_installed(&plan.target_dir, plugin_id)?;
    }
    let result = install_plugin(&plan).await;
    finish_with_cleanup(&plan.staging_dir, result).await
}

fn active_is_live_source(plugin_id: &str) -> bool {
    let Ok(config_dir) = crate::paths::shared_config_dir() else {
        return false;
    };
    let Ok(registry) = crate::plugins::registry::load_registry(&config_dir) else {
        return false;
    };
    registry
        .entries
        .iter()
        .find(|e| e.id == plugin_id)
        .map(|e| {
            matches!(
                e.active.source,
                crate::plugins::registry::SlotSource::DevLink { .. }
                    | crate::plugins::registry::SlotSource::WorktreeLink { .. }
            )
        })
        .unwrap_or(false)
}

async fn clear_stale_fallback(target_dir: &Path, plugin_id: &str) {
    if !target_dir.exists() {
        return;
    }
    log::info!(
        "Replacing stale release-asset fallback for dev-linked plugin: {}",
        plugin_id
    );
    if let Err(e) = tokio::fs::remove_dir_all(target_dir).await {
        log::warn!("Failed to clear stale fallback at {:?}: {}", target_dir, e);
    }
}

pub(super) async fn update(
    plugins_dir: &Path,
    repo_url: &str,
    plugin_id: &str,
    install_source: InstallSource,
) -> Result<()> {
    let plan = UpdatePlan::new(plugins_dir, repo_url, plugin_id, install_source);
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

struct InstallPlan<'a> {
    repo_url: &'a str,
    plugin_id: &'a str,
    install_source: InstallSource,
    staging_dir: PathBuf,
    target_dir: PathBuf,
}

impl<'a> InstallPlan<'a> {
    fn new(
        plugins_dir: &Path,
        repo_url: &'a str,
        plugin_id: &'a str,
        install_source: InstallSource,
    ) -> Self {
        Self {
            repo_url,
            plugin_id,
            install_source,
            staging_dir: install_staging_dir(plugins_dir, plugin_id),
            target_dir: plugins_dir.join(plugin_id),
        }
    }
}

struct UpdatePlan<'a> {
    repo_url: &'a str,
    plugin_id: &'a str,
    install_source: InstallSource,
    plugin_dir: PathBuf,
    staging_dir: PathBuf,
    backup_dir: PathBuf,
}

impl<'a> UpdatePlan<'a> {
    fn new(
        plugins_dir: &Path,
        repo_url: &'a str,
        plugin_id: &'a str,
        install_source: InstallSource,
    ) -> Self {
        Self {
            repo_url,
            plugin_id,
            install_source,
            plugin_dir: plugins_dir.join(plugin_id),
            staging_dir: update_staging_dir(plugins_dir, plugin_id),
            backup_dir: update_backup_dir(plugins_dir, plugin_id),
        }
    }
}

async fn install_plugin(plan: &InstallPlan<'_>) -> Result<()> {
    clone_plugin_repo(plan.repo_url, &plan.staging_dir, &plan.install_source).await?;
    install_dependencies(plan.plugin_id, &plan.staging_dir, &plan.install_source).await?;
    validate_staged_contract(&plan.staging_dir)?;
    finalize_install(&plan.staging_dir, &plan.target_dir).await?;
    log::info!("Plugin {} installed successfully", plan.plugin_id);
    Ok(())
}

async fn update_plugin(plan: &UpdatePlan<'_>) -> Result<()> {
    log::info!("Updating plugin {} from {}", plan.plugin_id, plan.repo_url);
    prepare_update_repo(
        plan.staging_dir.as_path(),
        plan.repo_url,
        &plan.install_source,
    )
    .await?;
    install_dependencies(plan.plugin_id, &plan.staging_dir, &plan.install_source).await?;
    validate_staged_contract(&plan.staging_dir)?;
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

fn validate_staged_contract(staging_dir: &Path) -> Result<()> {
    let manifest = crate::plugins::manifest::PluginManifest::load_and_validate(
        staging_dir.join("plugin.toml"),
    )?;
    manifest.plugin.require_declared_id()?;
    Ok(())
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
