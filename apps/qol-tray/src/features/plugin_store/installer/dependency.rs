use super::super::release_assets::{resolve_asset_pattern, PlatformTarget};
use super::InstallSource;
use anyhow::Result;
use std::path::{Path, PathBuf};

mod manifest;
mod release;
mod source_build;

use manifest::{load_plugin_manifest, validate_execution_contract};
use release::download_dependency_binary;
use source_build::{build_fallback_binary, set_executable_permissions};

pub(super) async fn install_dependencies(
    plugin_id: &str,
    plugin_dir: &Path,
    install_source: &InstallSource,
) -> Result<()> {
    let manifest = load_plugin_manifest(plugin_dir).await?;
    let installer = DependencyInstaller::new(plugin_id, plugin_dir, install_source);
    installer.install_manifest_binaries(&manifest).await?;
    validate_execution_contract(plugin_id, plugin_dir, &manifest)?;
    Ok(())
}

struct DependencyInstaller<'a> {
    plugin_id: &'a str,
    plugin_dir: &'a Path,
    install_source: &'a InstallSource,
}

impl<'a> DependencyInstaller<'a> {
    fn new(plugin_id: &'a str, plugin_dir: &'a Path, install_source: &'a InstallSource) -> Self {
        Self {
            plugin_id,
            plugin_dir,
            install_source,
        }
    }

    async fn install_manifest_binaries(
        &self,
        manifest: &crate::plugins::PluginManifest,
    ) -> Result<()> {
        let Some(dependencies) = manifest.dependencies.as_ref() else {
            return Ok(());
        };

        for dependency in &dependencies.binaries {
            self.install_binary(dependency).await?;
        }

        Ok(())
    }

    async fn install_binary(
        &self,
        dependency: &crate::plugins::manifest::BinaryDependency,
    ) -> Result<()> {
        validate_binary_name(dependency)?;
        let plan = DependencyPlan::new(
            self.plugin_id,
            self.plugin_dir,
            dependency,
            self.install_source,
        )?;
        ensure_dependency_binary(&plan).await?;
        set_executable_permissions(&plan.binary_path).await?;
        log::info!("Installed binary: {:?}", plan.binary_path);
        Ok(())
    }
}

pub(super) struct DependencyPlan<'a> {
    pub(super) plugin_id: &'a str,
    pub(super) plugin_dir: &'a Path,
    pub(super) dependency: &'a crate::plugins::manifest::BinaryDependency,
    pub(super) asset_name: String,
    pub(super) binary_path: PathBuf,
    pub(super) release_tag: Option<String>,
}

impl<'a> DependencyPlan<'a> {
    fn new(
        plugin_id: &'a str,
        plugin_dir: &'a Path,
        dependency: &'a crate::plugins::manifest::BinaryDependency,
        install_source: &InstallSource,
    ) -> Result<Self> {
        Ok(Self {
            plugin_id,
            plugin_dir,
            asset_name: resolve_asset_pattern(&dependency.pattern, PlatformTarget::current()?),
            binary_path: source_build::dependency_binary_output_path(plugin_dir, &dependency.name),
            dependency,
            release_tag: release_tag(install_source),
        })
    }

    pub(super) fn can_build_from_source_fallback(&self) -> bool {
        if !self.plugin_dir.join("Cargo.toml").is_file() {
            return false;
        }

        self.dependency
            .repo
            .eq_ignore_ascii_case(&format!("qol-tools/{}", self.plugin_id))
    }
}

fn release_tag(install_source: &InstallSource) -> Option<String> {
    let InstallSource::TaggedVersion(version) = install_source else {
        return None;
    };
    Some(format!("v{}", version))
}

async fn ensure_dependency_binary(plan: &DependencyPlan<'_>) -> Result<()> {
    if download_dependency_binary(plan).await? {
        return Ok(());
    }

    build_fallback_binary(plan).await
}

fn validate_binary_name(dependency: &crate::plugins::manifest::BinaryDependency) -> Result<()> {
    if crate::plugins::manifest::is_valid_command_basename(&dependency.name) {
        return Ok(());
    }

    anyhow::bail!(
        "Invalid dependency binary name {:?}; expected basename [A-Za-z0-9_-]",
        dependency.name
    )
}
