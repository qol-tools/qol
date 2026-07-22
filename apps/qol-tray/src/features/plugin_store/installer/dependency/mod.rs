use super::super::release_assets::{resolve_asset_pattern, PlatformTarget};
use super::super::source::PluginSource;
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
    source: &PluginSource,
    plugin_id: &str,
    plugin_dir: &Path,
    install_source: &InstallSource,
) -> Result<()> {
    let manifest = load_plugin_manifest(plugin_dir).await?;
    let installer = DependencyInstaller::new(source, plugin_id, plugin_dir, install_source);
    installer.install_manifest_binaries(&manifest).await?;
    validate_execution_contract(plugin_id, plugin_dir, &manifest)?;
    Ok(())
}

struct DependencyInstaller<'a> {
    source: &'a PluginSource,
    plugin_id: &'a str,
    plugin_dir: &'a Path,
    install_source: &'a InstallSource,
}

impl<'a> DependencyInstaller<'a> {
    fn new(
        source: &'a PluginSource,
        plugin_id: &'a str,
        plugin_dir: &'a Path,
        install_source: &'a InstallSource,
    ) -> Self {
        Self {
            source,
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
            self.source,
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
    pub(super) source: &'a PluginSource,
    pub(super) plugin_id: &'a str,
    pub(super) plugin_dir: &'a Path,
    pub(super) dependency: &'a crate::plugins::manifest::BinaryDependency,
    pub(super) asset_name: String,
    pub(super) binary_path: PathBuf,
    pub(super) release_tag: ReleaseTagPick,
}

#[derive(Debug, Clone)]
pub(super) enum ReleaseTagPick {
    Latest,
    PluginTag(String),
}

impl<'a> DependencyPlan<'a> {
    fn new(
        source: &'a PluginSource,
        plugin_id: &'a str,
        plugin_dir: &'a Path,
        dependency: &'a crate::plugins::manifest::BinaryDependency,
        install_source: &InstallSource,
    ) -> Result<Self> {
        Ok(Self {
            source,
            plugin_id,
            plugin_dir,
            asset_name: resolve_asset_pattern(&dependency.pattern, PlatformTarget::current()?),
            binary_path: source_build::dependency_binary_output_path(plugin_dir, &dependency.name),
            dependency,
            release_tag: release_tag(source, plugin_id, install_source),
        })
    }

    pub(super) fn asset_repo(&self) -> &str {
        &self.source.repo
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

fn release_tag(
    source: &PluginSource,
    plugin_id: &str,
    install_source: &InstallSource,
) -> ReleaseTagPick {
    match install_source {
        InstallSource::Latest => ReleaseTagPick::Latest,
        InstallSource::TaggedVersion(version) => {
            ReleaseTagPick::PluginTag(source.plugin_release_tag(plugin_id, version))
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::manifest::BinaryDependency;
    use std::path::PathBuf;

    fn binary_dep() -> BinaryDependency {
        BinaryDependency {
            name: "alt-tab".to_string(),
            repo: "qol-tools/plugin-alt-tab".to_string(),
            pattern: "alt-tab-{os}-{arch}".to_string(),
        }
    }

    fn core_source() -> PluginSource {
        PluginSource::new("core", "qol-tools/qol", "main")
    }

    #[test]
    fn release_tag_latest_for_latest_install_source() {
        let s = core_source();
        let pick = release_tag(&s, "plugin-alt-tab", &InstallSource::Latest);
        assert!(matches!(pick, ReleaseTagPick::Latest));
    }

    #[test]
    fn release_tag_uses_monorepo_plugin_prefix_for_tagged_install() {
        let s = core_source();
        let pick = release_tag(
            &s,
            "plugin-alt-tab",
            &InstallSource::TaggedVersion("1.2.3".to_string()),
        );
        match pick {
            ReleaseTagPick::PluginTag(tag) => assert_eq!(tag, "plugin-alt-tab-v1.2.3"),
            ReleaseTagPick::Latest => panic!("expected PluginTag for tagged install"),
        }
    }

    #[test]
    fn dependency_plan_overrides_asset_repo_to_source_not_manifest_dep_repo() {
        let s = core_source();
        let dep = binary_dep();
        let plugin_dir = PathBuf::from("/tmp/test-plugin");
        let plan = DependencyPlan::new(
            &s,
            "plugin-alt-tab",
            &plugin_dir,
            &dep,
            &InstallSource::TaggedVersion("1.2.3".to_string()),
        )
        .expect("plan constructs on the test host");
        assert_eq!(
            plan.asset_repo(),
            "qol-tools/qol",
            "asset repo must come from the SOURCE, not the manifest's legacy dependency.repo ({})",
            dep.repo
        );
        match &plan.release_tag {
            ReleaseTagPick::PluginTag(tag) => {
                assert_eq!(tag, "plugin-alt-tab-v1.2.3");
            }
            ReleaseTagPick::Latest => panic!("expected PluginTag"),
        }
    }
}
