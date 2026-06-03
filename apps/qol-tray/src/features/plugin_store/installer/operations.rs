use super::super::source::PluginSource;
use super::dependency::install_dependencies;
use super::source::{clone_source_repo, prepare_update_repo, resolve_latest_plugin_version};
use super::staging::{
    cleanup_temp_dir, install_staging_dir, swap_plugin_dirs, update_backup_dir, update_staging_dir,
};
use super::InstallSource;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub(super) async fn install(
    plugins_dir: &Path,
    source: &PluginSource,
    plugin_id: &str,
    install_source: InstallSource,
) -> Result<()> {
    let install_source = resolve_install_source(source, plugin_id, install_source).await?;
    let plan = InstallPlan::new(plugins_dir, source, plugin_id, install_source);
    if active_is_live_source(plugin_id) {
        clear_stale_fallback(&plan.target_dir, plugin_id).await;
    } else {
        ensure_not_installed(&plan.target_dir, plugin_id)?;
    }
    let result = install_plugin(&plan).await;
    finish_with_cleanup(&plan.clone_dir, &plan.extracted_dir, result).await
}

async fn resolve_install_source(
    source: &PluginSource,
    plugin_id: &str,
    install_source: InstallSource,
) -> Result<InstallSource> {
    match install_source {
        InstallSource::TaggedVersion(_) => Ok(install_source),
        InstallSource::Latest => {
            let version = resolve_latest_plugin_version(source, plugin_id).await?;
            Ok(InstallSource::TaggedVersion(version))
        }
    }
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
    source: &PluginSource,
    plugin_id: &str,
    install_source: InstallSource,
) -> Result<()> {
    let install_source = resolve_install_source(source, plugin_id, install_source).await?;
    let plan = UpdatePlan::new(plugins_dir, source, plugin_id, install_source);
    ensure_installed(&plan.plugin_dir, plugin_id)?;
    let result = update_plugin(&plan).await;
    finish_with_cleanup(&plan.clone_dir, &plan.extracted_dir, result).await
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
    source: &'a PluginSource,
    plugin_id: &'a str,
    install_source: InstallSource,
    clone_dir: PathBuf,
    extracted_dir: PathBuf,
    target_dir: PathBuf,
}

impl<'a> InstallPlan<'a> {
    fn new(
        plugins_dir: &Path,
        source: &'a PluginSource,
        plugin_id: &'a str,
        install_source: InstallSource,
    ) -> Self {
        let clone_dir = install_staging_dir(plugins_dir, plugin_id);
        let extracted_dir = extracted_plugin_path(&clone_dir);
        Self {
            source,
            plugin_id,
            install_source,
            clone_dir,
            extracted_dir,
            target_dir: plugins_dir.join(plugin_id),
        }
    }
}

struct UpdatePlan<'a> {
    source: &'a PluginSource,
    plugin_id: &'a str,
    install_source: InstallSource,
    plugin_dir: PathBuf,
    clone_dir: PathBuf,
    extracted_dir: PathBuf,
    backup_dir: PathBuf,
}

impl<'a> UpdatePlan<'a> {
    fn new(
        plugins_dir: &Path,
        source: &'a PluginSource,
        plugin_id: &'a str,
        install_source: InstallSource,
    ) -> Self {
        let clone_dir = update_staging_dir(plugins_dir, plugin_id);
        let extracted_dir = extracted_plugin_path(&clone_dir);
        Self {
            source,
            plugin_id,
            install_source,
            plugin_dir: plugins_dir.join(plugin_id),
            clone_dir,
            extracted_dir,
            backup_dir: update_backup_dir(plugins_dir, plugin_id),
        }
    }
}

fn extracted_plugin_path(clone_dir: &Path) -> PathBuf {
    let mut name = clone_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| ".extracted".to_string());
    name.push_str(".extracted");
    let parent = clone_dir.parent().unwrap_or(Path::new("."));
    parent.join(name)
}

async fn install_plugin(plan: &InstallPlan<'_>) -> Result<()> {
    clone_source_repo(
        plan.source,
        &plan.clone_dir,
        plan.plugin_id,
        &plan.install_source,
    )
    .await?;
    extract_plugin_subdir(&plan.clone_dir, &plan.extracted_dir, plan.plugin_id).await?;
    install_dependencies(
        plan.source,
        plan.plugin_id,
        &plan.extracted_dir,
        &plan.install_source,
    )
    .await?;
    validate_staged_contract(&plan.extracted_dir)?;
    finalize_install(&plan.extracted_dir, &plan.target_dir).await?;
    log::info!("Plugin {} installed successfully", plan.plugin_id);
    Ok(())
}

async fn update_plugin(plan: &UpdatePlan<'_>) -> Result<()> {
    log::info!(
        "Updating plugin {} from source {} ({})",
        plan.plugin_id,
        plan.source.name,
        plan.source.repo
    );
    prepare_update_repo(
        plan.source,
        plan.clone_dir.as_path(),
        plan.plugin_id,
        &plan.install_source,
    )
    .await?;
    extract_plugin_subdir(&plan.clone_dir, &plan.extracted_dir, plan.plugin_id).await?;
    install_dependencies(
        plan.source,
        plan.plugin_id,
        &plan.extracted_dir,
        &plan.install_source,
    )
    .await?;
    validate_staged_contract(&plan.extracted_dir)?;
    swap_plugin_dirs(&plan.plugin_dir, &plan.extracted_dir, &plan.backup_dir).await?;
    log::info!("Plugin {} updated successfully", plan.plugin_id);
    Ok(())
}

async fn extract_plugin_subdir(
    clone_dir: &Path,
    extracted_dir: &Path,
    plugin_id: &str,
) -> Result<()> {
    let source_subdir = clone_dir.join("plugins").join(plugin_id);
    if !source_subdir.is_dir() {
        anyhow::bail!(
            "Cloned source does not contain plugins/{}/ at {:?}",
            plugin_id,
            clone_dir
        );
    }
    let manifest_path = source_subdir.join("plugin.toml");
    if !manifest_path.is_file() {
        anyhow::bail!(
            "Cloned source is missing plugins/{}/plugin.toml at {:?}",
            plugin_id,
            clone_dir
        );
    }
    if extracted_dir.exists() {
        tokio::fs::remove_dir_all(extracted_dir)
            .await
            .with_context(|| format!("Failed to clear stale extracted dir {:?}", extracted_dir))?;
    }
    tokio::fs::rename(&source_subdir, extracted_dir)
        .await
        .with_context(|| {
            format!(
                "Failed to extract plugins/{}/ from {:?} to {:?}",
                plugin_id, clone_dir, extracted_dir
            )
        })?;
    Ok(())
}

async fn finish_with_cleanup(
    clone_dir: &Path,
    extracted_dir: &Path,
    result: Result<()>,
) -> Result<()> {
    cleanup_temp_dir(clone_dir).await;
    if result.is_err() {
        cleanup_temp_dir(extracted_dir).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn extract_plugin_subdir_moves_plugin_dir_out_of_clone() {
        let temp = TempDir::new().unwrap();
        let clone = temp.path().join("clone");
        let plugin_subdir = clone.join("plugins").join("plugin-alt-tab");
        tokio::fs::create_dir_all(&plugin_subdir).await.unwrap();
        tokio::fs::write(plugin_subdir.join("plugin.toml"), b"[plugin]\nname=\"x\"\n")
            .await
            .unwrap();
        tokio::fs::create_dir_all(clone.join("apps").join("qol-tray"))
            .await
            .unwrap();

        let extracted = temp.path().join("extracted");
        extract_plugin_subdir(&clone, &extracted, "plugin-alt-tab")
            .await
            .unwrap();

        assert!(extracted.join("plugin.toml").is_file());
        assert!(
            !clone.join("plugins").join("plugin-alt-tab").exists(),
            "subdir must be moved, not copied"
        );
        assert!(
            clone.join("apps").join("qol-tray").is_dir(),
            "other monorepo dirs remain in the clone"
        );
    }

    #[tokio::test]
    async fn extract_plugin_subdir_errors_when_subdir_missing() {
        let temp = TempDir::new().unwrap();
        let clone = temp.path().join("clone");
        tokio::fs::create_dir_all(&clone).await.unwrap();
        let extracted = temp.path().join("extracted");
        let err = extract_plugin_subdir(&clone, &extracted, "plugin-missing")
            .await
            .unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.contains("plugins/plugin-missing/"),
            "error must name the missing subdir, got: {}",
            msg
        );
    }

    #[tokio::test]
    async fn extract_plugin_subdir_errors_when_manifest_missing() {
        let temp = TempDir::new().unwrap();
        let clone = temp.path().join("clone");
        let plugin_subdir = clone.join("plugins").join("plugin-alt-tab");
        tokio::fs::create_dir_all(&plugin_subdir).await.unwrap();
        let extracted = temp.path().join("extracted");
        let err = extract_plugin_subdir(&clone, &extracted, "plugin-alt-tab")
            .await
            .unwrap_err();
        let msg = format!("{}", err);
        assert!(msg.contains("plugin.toml"), "error: {}", msg);
    }
}
