use anyhow::{Context, Result};
use std::path::PathBuf;
use std::time::Duration;

use super::source::PluginSource;

mod command;
mod dependency;
mod lock;
mod operation_lock;
mod operations;
mod source;
mod staging;

#[doc(hidden)]
pub mod testing;

const GIT_TIMEOUT: Duration = Duration::from_secs(120);
const DEFAULT_CARGO_BUILD_JOBS: usize = 4;
const CARGO_BUILD_TIMEOUT: Duration = Duration::from_secs(300);
#[cfg(unix)]
const LOCKFILE_MAX_AGE: Duration = Duration::from_secs(30);
#[cfg(not(unix))]
const LOCKFILE_MAX_AGE: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
pub(super) enum InstallSource {
    Latest,
    TaggedVersion(String),
}

pub(crate) struct PluginInstaller {
    plugins_dir: PathBuf,
}

impl PluginInstaller {
    pub(crate) fn new(plugins_dir: PathBuf) -> Self {
        Self { plugins_dir }
    }

    pub(crate) async fn install(&self, source: &PluginSource, plugin_id: &str) -> Result<()> {
        validate_plugin_id(plugin_id)?;
        let _operation_lock = operation_lock::acquire_operation_lock(&self.plugins_dir, plugin_id)?;
        operations::install(
            &self.plugins_dir,
            source,
            plugin_id,
            InstallSource::Latest,
        )
        .await?;
        self.record_release_install(plugin_id)
    }

    pub(crate) async fn install_exact(
        &self,
        source: &PluginSource,
        plugin_id: &str,
        version: &str,
    ) -> Result<()> {
        validate_plugin_id(plugin_id)?;
        let _operation_lock = operation_lock::acquire_operation_lock(&self.plugins_dir, plugin_id)?;
        operations::install(
            &self.plugins_dir,
            source,
            plugin_id,
            InstallSource::TaggedVersion(version.to_string()),
        )
        .await?;
        self.record_release_install(plugin_id)
    }

    pub(crate) async fn update(&self, source: &PluginSource, plugin_id: &str) -> Result<()> {
        validate_plugin_id(plugin_id)?;
        let _operation_lock = operation_lock::acquire_operation_lock(&self.plugins_dir, plugin_id)?;
        operations::update(
            &self.plugins_dir,
            source,
            plugin_id,
            InstallSource::Latest,
        )
        .await?;
        self.record_release_install(plugin_id)
    }

    pub(crate) async fn update_exact(
        &self,
        source: &PluginSource,
        plugin_id: &str,
        version: &str,
    ) -> Result<()> {
        validate_plugin_id(plugin_id)?;
        let _operation_lock = operation_lock::acquire_operation_lock(&self.plugins_dir, plugin_id)?;
        operations::update(
            &self.plugins_dir,
            source,
            plugin_id,
            InstallSource::TaggedVersion(version.to_string()),
        )
        .await?;
        self.record_release_install(plugin_id)
    }

    pub(crate) async fn uninstall(&self, plugin_id: &str) -> Result<()> {
        validate_plugin_id(plugin_id)?;
        let _operation_lock = operation_lock::acquire_operation_lock(&self.plugins_dir, plugin_id)?;
        operations::uninstall(&self.plugins_dir, plugin_id).await?;
        self.record_release_uninstall(plugin_id)
    }

    fn record_release_install(&self, plugin_id: &str) -> Result<()> {
        let config_dir = crate::paths::shared_config_dir()?;
        let plugin_root = self.plugins_dir.join(plugin_id);
        crate::plugins::registry::record_release_install(&config_dir, plugin_id, plugin_root)
            .map_err(|e| anyhow::anyhow!("{}", e))
            .context("Failed to update plugin registry after install")
    }

    fn record_release_uninstall(&self, plugin_id: &str) -> Result<()> {
        let config_dir = crate::paths::shared_config_dir()?;
        crate::plugins::registry::record_release_uninstall(&config_dir, plugin_id)
            .map_err(|e| anyhow::anyhow!("{}", e))
            .context("Failed to update plugin registry after uninstall")
    }

    pub(crate) async fn load_source_config_contract(
        &self,
        source: &PluginSource,
        plugin_id: &str,
        version: Option<&str>,
    ) -> Result<Option<qol_config::contract::ConfigSpec>> {
        validate_plugin_id(plugin_id)?;
        let staging_dir = staging::install_staging_dir(&self.plugins_dir, plugin_id);
        let install_source = match version {
            Some(version) if !version.is_empty() => {
                InstallSource::TaggedVersion(version.to_string())
            }
            _ => InstallSource::Latest,
        };
        let result = source::clone_source_repo(source, &staging_dir, plugin_id, &install_source)
            .await
            .and_then(|_| {
                let plugin_subdir = staging_dir.join("plugins").join(plugin_id);
                if !plugin_subdir.is_dir() {
                    anyhow::bail!(
                        "Cloned source {} does not contain plugins/{}/",
                        source.repo,
                        plugin_id
                    );
                }
                crate::plugins::config::load_config_contract_from_root(&plugin_subdir)
            });
        staging::cleanup_temp_dir(&staging_dir).await;
        result
    }
}

fn validate_plugin_id(plugin_id: &str) -> Result<()> {
    if super::validation::is_safe_plugin_id(plugin_id) {
        return Ok(());
    }
    anyhow::bail!("{}: {}", super::validation::INVALID_PLUGIN_ID, plugin_id)
}
