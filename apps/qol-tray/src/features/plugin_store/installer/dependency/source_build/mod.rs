mod artifact;
mod manifest_sanitizer;

use super::super::command::run_cargo_build;
use super::DependencyPlan;
use anyhow::Result;
use std::path::Path;

pub(super) use artifact::{dependency_binary_output_path, set_executable_permissions};

pub(super) async fn build_fallback_binary(plan: &DependencyPlan<'_>) -> Result<()> {
    ensure_source_fallback_available(plan)?;
    log::warn!(
        "Falling back to local source build for dependency {} in {:?}",
        plan.dependency.name,
        plan.plugin_dir
    );
    build_binary_from_source(plan.plugin_dir, &plan.dependency.name).await
}

fn ensure_source_fallback_available(plan: &DependencyPlan<'_>) -> Result<()> {
    if plan.can_build_from_source_fallback() {
        return Ok(());
    }

    anyhow::bail!(
        "Asset '{}' not available for {} and source-build fallback is unavailable",
        plan.asset_name,
        plan.dependency.repo
    )
}

async fn build_binary_from_source(plugin_dir: &Path, binary_name: &str) -> Result<()> {
    let manifest_path = plugin_dir.join("Cargo.toml");
    manifest_sanitizer::ensure_release_build_manifest(&manifest_path)?;
    let output = run_cargo_build(&manifest_path, plugin_dir).await?;
    ensure_build_succeeded(&output)?;
    let source_path = artifact::built_binary_path(plugin_dir, binary_name)?;
    let output_path = artifact::dependency_binary_output_path(plugin_dir, binary_name);
    artifact::install_built_binary(&source_path, &output_path).await
}

fn ensure_build_succeeded(output: &std::process::Output) -> Result<()> {
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    anyhow::bail!("Cargo build failed: {}", stderr.trim())
}
