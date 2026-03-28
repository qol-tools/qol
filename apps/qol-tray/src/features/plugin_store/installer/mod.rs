use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;

mod command;
mod dependency;
mod lock;
mod operation_lock;
mod operations;
mod source;
mod staging;

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

    pub(crate) async fn install(&self, repo_url: &str, plugin_id: &str) -> Result<()> {
        validate_plugin_id(plugin_id)?;
        let _operation_lock = operation_lock::acquire_operation_lock(&self.plugins_dir, plugin_id)?;
        operations::ensure_no_dev_link_conflict(plugin_id)?;
        operations::install(
            &self.plugins_dir,
            repo_url,
            plugin_id,
            InstallSource::Latest,
        )
        .await
    }

    pub(crate) async fn install_exact(
        &self,
        repo_url: &str,
        plugin_id: &str,
        version: &str,
    ) -> Result<()> {
        validate_plugin_id(plugin_id)?;
        let _operation_lock = operation_lock::acquire_operation_lock(&self.plugins_dir, plugin_id)?;
        operations::ensure_no_dev_link_conflict(plugin_id)?;
        operations::install(
            &self.plugins_dir,
            repo_url,
            plugin_id,
            InstallSource::TaggedVersion(version.to_string()),
        )
        .await
    }

    pub(crate) async fn update(&self, repo_url: &str, plugin_id: &str) -> Result<()> {
        validate_plugin_id(plugin_id)?;
        let _operation_lock = operation_lock::acquire_operation_lock(&self.plugins_dir, plugin_id)?;
        operations::ensure_no_dev_link_conflict(plugin_id)?;
        operations::update(
            &self.plugins_dir,
            repo_url,
            plugin_id,
            InstallSource::Latest,
        )
        .await
    }

    pub(crate) async fn update_exact(
        &self,
        repo_url: &str,
        plugin_id: &str,
        version: &str,
    ) -> Result<()> {
        validate_plugin_id(plugin_id)?;
        let _operation_lock = operation_lock::acquire_operation_lock(&self.plugins_dir, plugin_id)?;
        operations::ensure_no_dev_link_conflict(plugin_id)?;
        operations::update(
            &self.plugins_dir,
            repo_url,
            plugin_id,
            InstallSource::TaggedVersion(version.to_string()),
        )
        .await
    }

    pub(crate) async fn uninstall(&self, plugin_id: &str) -> Result<()> {
        validate_plugin_id(plugin_id)?;
        let _operation_lock = operation_lock::acquire_operation_lock(&self.plugins_dir, plugin_id)?;
        operations::uninstall(&self.plugins_dir, plugin_id).await
    }
}

fn validate_plugin_id(plugin_id: &str) -> Result<()> {
    if super::validation::is_safe_plugin_id(plugin_id) {
        return Ok(());
    }
    anyhow::bail!("{}: {}", super::validation::INVALID_PLUGIN_ID, plugin_id)
}
