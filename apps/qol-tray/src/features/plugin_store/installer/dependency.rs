use super::super::release_assets::{resolve_asset_pattern, PlatformTarget};
use super::command::run_cargo_build;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub(super) async fn install_dependencies(plugin_id: &str, plugin_dir: &Path) -> Result<()> {
    let manifest = validated_manifest(plugin_dir).await?;
    install_manifest_binaries(plugin_id, plugin_dir, &manifest).await?;
    validate_execution_contract(plugin_id, plugin_dir, &manifest)?;
    Ok(())
}

async fn validated_manifest(plugin_dir: &Path) -> Result<crate::plugins::PluginManifest> {
    let manifest_path = plugin_dir.join("plugin.toml");
    ensure_manifest_exists(&manifest_path)?;
    let content = read_manifest(&manifest_path).await?;
    parse_manifest(&content)
}

fn ensure_manifest_exists(manifest_path: &Path) -> Result<()> {
    if manifest_path.exists() {
        return Ok(());
    }
    anyhow::bail!("Missing plugin.toml in {}", manifest_path.display())
}

async fn read_manifest(manifest_path: &Path) -> Result<String> {
    tokio::fs::read_to_string(manifest_path)
        .await
        .with_context(|| format!("Failed to read {}", manifest_path.display()))
}

fn parse_manifest(content: &str) -> Result<crate::plugins::PluginManifest> {
    let manifest: crate::plugins::PluginManifest =
        toml::from_str(content).context("Failed to parse plugin.toml")?;
    manifest
        .validate()
        .context("Invalid plugin.toml contract")?;
    Ok(manifest)
}

async fn install_manifest_binaries(
    plugin_id: &str,
    plugin_dir: &Path,
    manifest: &crate::plugins::PluginManifest,
) -> Result<()> {
    let Some(dependencies) = manifest.dependencies.as_ref() else {
        return Ok(());
    };

    for dependency in &dependencies.binaries {
        install_binary(plugin_id, plugin_dir, dependency).await?;
    }

    Ok(())
}

fn validate_execution_contract(
    plugin_id: &str,
    plugin_dir: &Path,
    manifest: &crate::plugins::PluginManifest,
) -> Result<()> {
    if !manifest.plugin.supports_current_platform() {
        return Ok(());
    }
    crate::plugins::validate_execution_contract(plugin_id, manifest, plugin_dir)
        .context("Plugin binary contract preflight failed")
}

async fn install_binary(
    plugin_id: &str,
    plugin_dir: &Path,
    dependency: &crate::plugins::manifest::BinaryDependency,
) -> Result<()> {
    validate_binary_name(dependency)?;
    let plan = DependencyPlan::new(plugin_id, plugin_dir, dependency)?;
    ensure_dependency_binary(&plan).await?;
    set_executable_permissions(&plan.binary_path).await?;
    log::info!("Installed binary: {:?}", plan.binary_path);
    Ok(())
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

struct DependencyPlan<'a> {
    plugin_id: &'a str,
    plugin_dir: &'a Path,
    dependency: &'a crate::plugins::manifest::BinaryDependency,
    asset_name: String,
    binary_path: PathBuf,
}

impl<'a> DependencyPlan<'a> {
    fn new(
        plugin_id: &'a str,
        plugin_dir: &'a Path,
        dependency: &'a crate::plugins::manifest::BinaryDependency,
    ) -> Result<Self> {
        Ok(Self {
            plugin_id,
            plugin_dir,
            asset_name: resolve_asset_pattern(&dependency.pattern, PlatformTarget::current()?),
            binary_path: dependency_binary_output_path(plugin_dir, &dependency.name),
            dependency,
        })
    }
}

async fn ensure_dependency_binary(plan: &DependencyPlan<'_>) -> Result<()> {
    if download_dependency_binary(plan).await? {
        return Ok(());
    }
    build_fallback_binary(plan).await
}

async fn download_dependency_binary(plan: &DependencyPlan<'_>) -> Result<bool> {
    log::info!("Fetching {} from {}", plan.asset_name, plan.dependency.repo);

    let release = match fetch_latest_release(&plan.dependency.repo).await {
        Ok(release) => release,
        Err(error) => return release_fetch_fallback(plan, &error),
    };

    let Some(asset) = release
        .assets
        .iter()
        .find(|asset| asset.name == plan.asset_name)
    else {
        log::warn!(
            "Release asset '{}' missing for {}",
            plan.asset_name,
            plan.dependency.repo
        );
        return Ok(false);
    };

    download_asset(&asset.browser_download_url, &plan.binary_path).await?;
    Ok(true)
}

fn release_fetch_fallback(plan: &DependencyPlan<'_>, error: &anyhow::Error) -> Result<bool> {
    log::warn!(
        "Failed to fetch release asset {} from {}: {:#}",
        plan.asset_name,
        plan.dependency.repo,
        error
    );
    Ok(false)
}

async fn build_fallback_binary(plan: &DependencyPlan<'_>) -> Result<()> {
    ensure_source_fallback_available(plan)?;
    log::warn!(
        "Falling back to local source build for dependency {} in {:?}",
        plan.dependency.name,
        plan.plugin_dir
    );
    build_binary_from_source(plan.plugin_dir, &plan.dependency.name).await
}

fn ensure_source_fallback_available(plan: &DependencyPlan<'_>) -> Result<()> {
    if can_build_from_source_fallback(plan) {
        return Ok(());
    }
    anyhow::bail!(
        "Asset '{}' not available for {} and source-build fallback is unavailable",
        plan.asset_name,
        plan.dependency.repo
    )
}

fn can_build_from_source_fallback(plan: &DependencyPlan<'_>) -> bool {
    if !plan.plugin_dir.join("Cargo.toml").is_file() {
        return false;
    }

    let expected_repo = format!("qol-tools/{}", plan.plugin_id);
    plan.dependency.repo.eq_ignore_ascii_case(&expected_repo)
}

fn dependency_binary_output_path(plugin_dir: &Path, binary_name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        if std::path::Path::new(binary_name).extension().is_none() {
            return plugin_dir.join(format!("{}.exe", binary_name));
        }
    }

    plugin_dir.join(binary_name)
}

fn built_binary_candidates(plugin_dir: &Path, binary_name: &str) -> Vec<PathBuf> {
    let release_dir = plugin_dir.join("target").join("release");

    #[cfg(windows)]
    {
        let mut candidates = vec![release_dir.join(binary_name)];
        if std::path::Path::new(binary_name).extension().is_none() {
            candidates.push(release_dir.join(format!("{}.exe", binary_name)));
        }
        candidates
    }

    #[cfg(not(windows))]
    {
        vec![release_dir.join(binary_name)]
    }
}

async fn install_built_binary(source_path: &Path, output_path: &Path) -> Result<()> {
    let staged_path = output_path.with_extension("new");
    let _ = tokio::fs::remove_file(&staged_path).await;

    tokio::fs::copy(source_path, &staged_path)
        .await
        .with_context(|| {
            format!(
                "Failed to stage built binary {} -> {}",
                source_path.display(),
                staged_path.display()
            )
        })?;

    tokio::fs::rename(&staged_path, output_path)
        .await
        .with_context(|| {
            format!(
                "Failed to install built binary {} -> {}",
                staged_path.display(),
                output_path.display()
            )
        })?;

    Ok(())
}

async fn build_binary_from_source(plugin_dir: &Path, binary_name: &str) -> Result<()> {
    let manifest_path = plugin_dir.join("Cargo.toml");
    ensure_cargo_manifest(&manifest_path)?;
    strip_dev_dependencies_for_release_build(&manifest_path)?;

    let output = run_cargo_build(&manifest_path, plugin_dir).await?;
    ensure_build_succeeded(&output)?;

    let source_path = built_binary_candidates(plugin_dir, binary_name)
        .into_iter()
        .find(|path| path.is_file())
        .with_context(|| {
            format!(
                "Built binary not found for {} in {}",
                binary_name,
                plugin_dir.join("target").join("release").display()
            )
        })?;

    let output_path = dependency_binary_output_path(plugin_dir, binary_name);
    install_built_binary(&source_path, &output_path).await
}

fn ensure_cargo_manifest(manifest_path: &Path) -> Result<()> {
    if manifest_path.is_file() {
        return Ok(());
    }
    anyhow::bail!("Cargo.toml not found at {}", manifest_path.display())
}

fn ensure_build_succeeded(output: &std::process::Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!("Cargo build failed: {}", stderr.trim())
}

fn strip_dev_dependencies_for_release_build(manifest_path: &Path) -> Result<()> {
    let content = std::fs::read_to_string(manifest_path)
        .with_context(|| format!("Failed to read {}", manifest_path.display()))?;
    let mut value: toml::Value = toml::from_str(&content)
        .with_context(|| format!("Failed to parse {}", manifest_path.display()))?;
    let Some(root) = value.as_table_mut() else {
        return Ok(());
    };

    let mut changed = root.remove("dev-dependencies").is_some();
    if let Some(target) = root
        .get_mut("target")
        .and_then(|value| value.as_table_mut())
    {
        for target_entry in target.values_mut() {
            let Some(target_table) = target_entry.as_table_mut() else {
                continue;
            };
            if target_table.remove("dev-dependencies").is_some() {
                changed = true;
            }
        }
    }

    if !changed {
        return Ok(());
    }

    let rendered = toml::to_string(&value)
        .with_context(|| format!("Failed to render {}", manifest_path.display()))?;
    std::fs::write(manifest_path, rendered)
        .with_context(|| format!("Failed to write {}", manifest_path.display()))?;
    log::info!(
        "Stripped dev-dependencies from {} for install build",
        manifest_path.display()
    );
    Ok(())
}

#[derive(Deserialize)]
struct GitHubRelease {
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

async fn fetch_latest_release(repo: &str) -> Result<GitHubRelease> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    let response = github_request(&url).await?;
    Ok(response.json().await?)
}

async fn download_asset(url: &str, dest: &Path) -> Result<()> {
    let response = github_request(url).await?;
    let bytes = response.bytes().await?;
    tokio::fs::write(dest, &bytes).await?;
    Ok(())
}

async fn github_request(url: &str) -> Result<reqwest::Response> {
    let client = reqwest::Client::new();
    let token = super::super::github::get_stored_token();
    let request = super::super::github::build_github_request(&client, url, token.as_deref());
    super::super::github::send_checked(request).await
}

#[cfg(unix)]
async fn set_executable_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = tokio::fs::metadata(path).await?.permissions();
    permissions.set_mode(0o755);
    tokio::fs::set_permissions(path, permissions).await?;
    Ok(())
}

#[cfg(not(unix))]
async fn set_executable_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn strip_dev_dependencies_for_release_build_removes_top_level_and_target_sections() {
        let temp = TempDir::new().unwrap();
        let manifest_path = temp.path().join("Cargo.toml");

        std::fs::write(
            &manifest_path,
            r#"
[package]
name = "sample"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1"

[dev-dependencies]
qol-tray = { path = "../../qol-tray" }
toml = "0.9"

[target.'cfg(unix)'.dev-dependencies]
tempfile = "3"
"#,
        )
        .unwrap();

        strip_dev_dependencies_for_release_build(&manifest_path).unwrap();

        let after = std::fs::read_to_string(&manifest_path).unwrap();
        assert!(!after.contains("dev-dependencies"));
        assert!(after.contains("serde"));
    }
}
