use super::super::index::IndexedRelease;
use super::super::release_assets::{resolve_asset_pattern, PlatformTarget};
use super::super::source::PluginSource;
use super::InstallSource;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::time::Duration;

mod manifest;
mod probe;
mod release;
mod source_build;

use manifest::{load_plugin_manifest, validate_execution_contract};
use release::download_dependency_binary;
use source_build::build_fallback_binary;

pub(super) async fn set_executable_permissions(path: &Path) -> Result<()> {
    source_build::set_executable_permissions(path).await
}

const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy)]
pub(super) enum AssetOrigin<'a> {
    GitHubRelease,
    Index(&'a IndexedRelease),
}

pub(super) async fn install_dependencies(
    source: &PluginSource,
    plugin_id: &str,
    plugin_dir: &Path,
    install_source: &InstallSource,
    origin: AssetOrigin<'_>,
) -> Result<()> {
    let manifest = load_plugin_manifest(plugin_dir).await?;
    let installer = DependencyInstaller::new(source, plugin_id, plugin_dir, install_source, origin);
    installer.install_manifest_binaries(&manifest).await?;
    validate_execution_contract(plugin_id, plugin_dir, &manifest)?;
    Ok(())
}

struct DependencyInstaller<'a> {
    source: &'a PluginSource,
    plugin_id: &'a str,
    plugin_dir: &'a Path,
    install_source: &'a InstallSource,
    origin: AssetOrigin<'a>,
}

impl<'a> DependencyInstaller<'a> {
    fn new(
        source: &'a PluginSource,
        plugin_id: &'a str,
        plugin_dir: &'a Path,
        install_source: &'a InstallSource,
        origin: AssetOrigin<'a>,
    ) -> Self {
        Self {
            source,
            plugin_id,
            plugin_dir,
            install_source,
            origin,
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
        let mut plan = DependencyPlan::new(
            self.source,
            self.plugin_id,
            self.plugin_dir,
            dependency,
            self.install_source,
            self.origin,
        )?;
        let live_path = plan.binary_path.clone();
        let staged_path = live_path.with_extension("new");
        let _ = tokio::fs::remove_file(&staged_path).await;
        plan.binary_path = staged_path.clone();
        ensure_dependency_binary(&plan).await?;
        set_executable_permissions(&staged_path).await?;
        promote_candidate(&staged_path, &live_path, self.plugin_id, &dependency.name).await?;
        log::info!("Installed binary: {:?}", live_path);
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
    pub(super) origin: AssetOrigin<'a>,
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
        origin: AssetOrigin<'a>,
    ) -> Result<Self> {
        Ok(Self {
            source,
            plugin_id,
            plugin_dir,
            asset_name: resolve_asset_pattern(&dependency.pattern, PlatformTarget::current()?),
            binary_path: source_build::dependency_binary_output_path(plugin_dir, &dependency.name),
            dependency,
            release_tag: release_tag(source, plugin_id, install_source),
            origin,
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

async fn promote_candidate(
    staged: &Path,
    live: &Path,
    plugin_id: &str,
    binary_name: &str,
) -> Result<()> {
    promote_candidate_with_timeout(staged, live, plugin_id, binary_name, PROBE_TIMEOUT).await
}

async fn promote_candidate_with_timeout(
    staged: &Path,
    live: &Path,
    plugin_id: &str,
    binary_name: &str,
    timeout: Duration,
) -> Result<()> {
    let candidate = staged.to_path_buf();
    let probe =
        tokio::task::spawn_blocking(move || probe::verify_candidate_runs(&candidate, timeout))
            .await
            .context("Plugin compatibility probe task failed")?;
    if let Err(error) = probe {
        let _ = tokio::fs::remove_file(staged).await;
        return Err(error)
            .with_context(|| format!("Cannot install {} for plugin {}", binary_name, plugin_id));
    }
    tokio::fs::rename(staged, live)
        .await
        .with_context(|| format!("Failed to install {} at {}", binary_name, live.display()))
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
            name: "qol-alt-tab".to_string(),
            repo: "qol-tools/plugin-alt-tab".to_string(),
            pattern: "qol-alt-tab-{os}-{arch}".to_string(),
        }
    }

    fn core_source() -> PluginSource {
        PluginSource::new("core", "qol-tools/qol", "main")
    }

    #[test]
    fn release_tag_latest_for_latest_install_source() {
        let s = core_source();
        let pick = release_tag(&s, "qol-alt-tab", &InstallSource::Latest);
        assert!(matches!(pick, ReleaseTagPick::Latest));
    }

    #[test]
    fn release_tag_uses_monorepo_plugin_prefix_for_tagged_install() {
        let s = core_source();
        let pick = release_tag(
            &s,
            "qol-alt-tab",
            &InstallSource::TaggedVersion("1.2.3".to_string()),
        );
        match pick {
            ReleaseTagPick::PluginTag(tag) => assert_eq!(tag, "qol-alt-tab-v1.2.3"),
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
            "qol-alt-tab",
            &plugin_dir,
            &dep,
            &InstallSource::TaggedVersion("1.2.3".to_string()),
            AssetOrigin::GitHubRelease,
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
                assert_eq!(tag, "qol-alt-tab-v1.2.3");
            }
            ReleaseTagPick::Latest => panic!("expected PluginTag"),
        }
    }

    #[cfg(unix)]
    fn write_candidate(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::write(path, body).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_candidate_that_starts_replaces_the_installed_binary() {
        for body in ["exit 0", "exit 1"] {
            let dir = tempfile::tempdir().unwrap();
            let live = dir.path().join("qol-alt-tab");
            let staged = live.with_extension("new");
            std::fs::write(&live, b"working installation").unwrap();
            write_candidate(
                &staged,
                &format!("#!/bin/sh\nprintf '%s' \"$0\" > \"$(dirname \"$0\")/probed\"\n{body}\n"),
            );
            let expected = std::fs::read(&staged).unwrap();

            promote_candidate_with_timeout(
                &staged,
                &live,
                "qol-alt-tab",
                "qol-alt-tab",
                PROBE_TIMEOUT,
            )
            .await
            .unwrap_or_else(|error| panic!("body={body}: {error:#}"));

            assert_eq!(std::fs::read(&live).unwrap(), expected, "body={body}");
            assert_eq!(
                std::fs::read_to_string(dir.path().join("probed")).unwrap(),
                staged.display().to_string(),
                "body={body}"
            );
            assert!(!staged.exists(), "body={body}");
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_candidate_that_cannot_run_keeps_the_installed_binary() {
        for content in ["#!/missing/loader\n", "#!/bin/sh\nexec sleep 30\n"] {
            let dir = tempfile::tempdir().unwrap();
            let live = dir.path().join("qol-alt-tab");
            let staged = live.with_extension("new");
            std::fs::write(&live, b"working installation").unwrap();
            write_candidate(&staged, content);

            let error = promote_candidate_with_timeout(
                &staged,
                &live,
                "qol-alt-tab",
                "qol-alt-tab",
                std::time::Duration::from_millis(50),
            )
            .await
            .unwrap_err();

            assert!(
                format!("{error:#}").contains("cannot run on this system"),
                "content={content}: {error:#}"
            );
            assert_eq!(
                std::fs::read(&live).unwrap(),
                b"working installation",
                "content={content}"
            );
            assert!(!staged.exists(), "content={content}");
        }
    }
}
