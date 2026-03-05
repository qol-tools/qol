use super::super::command::run_cargo_build;
use super::DependencyPlan;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub(super) async fn build_fallback_binary(plan: &DependencyPlan<'_>) -> Result<()> {
    ensure_source_fallback_available(plan)?;
    log::warn!(
        "Falling back to local source build for dependency {} in {:?}",
        plan.dependency.name,
        plan.plugin_dir
    );
    build_binary_from_source(plan.plugin_dir, &plan.dependency.name).await
}

pub(super) fn dependency_binary_output_path(plugin_dir: &Path, binary_name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        if Path::new(binary_name).extension().is_none() {
            return plugin_dir.join(format!("{}.exe", binary_name));
        }
    }

    plugin_dir.join(binary_name)
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
    ensure_cargo_manifest(&manifest_path)?;
    strip_dev_dependencies_for_release_build(&manifest_path)?;
    let output = run_cargo_build(&manifest_path, plugin_dir).await?;
    ensure_build_succeeded(&output)?;
    let source_path = built_binary_path(plugin_dir, binary_name)?;
    let output_path = dependency_binary_output_path(plugin_dir, binary_name);
    install_built_binary(&source_path, &output_path).await
}

fn built_binary_path(plugin_dir: &Path, binary_name: &str) -> Result<PathBuf> {
    built_binary_candidates(plugin_dir, binary_name)
        .into_iter()
        .find(|path| path.is_file())
        .with_context(|| missing_built_binary(binary_name, plugin_dir))
}

fn missing_built_binary(binary_name: &str, plugin_dir: &Path) -> String {
    format!(
        "Built binary not found for {} in {}",
        binary_name,
        plugin_dir.join("target").join("release").display()
    )
}

fn built_binary_candidates(plugin_dir: &Path, binary_name: &str) -> Vec<PathBuf> {
    let release_dir = plugin_dir.join("target").join("release");

    #[cfg(windows)]
    {
        let mut candidates = vec![release_dir.join(binary_name)];
        if Path::new(binary_name).extension().is_none() {
            candidates.push(release_dir.join(format!("{}.exe", binary_name)));
        }
        return candidates;
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
        .with_context(|| stage_copy_error(source_path, &staged_path))?;

    tokio::fs::rename(&staged_path, output_path)
        .await
        .with_context(|| install_copy_error(&staged_path, output_path))?;

    Ok(())
}

fn stage_copy_error(source_path: &Path, staged_path: &Path) -> String {
    format!(
        "Failed to stage built binary {} -> {}",
        source_path.display(),
        staged_path.display()
    )
}

fn install_copy_error(staged_path: &Path, output_path: &Path) -> String {
    format!(
        "Failed to install built binary {} -> {}",
        staged_path.display(),
        output_path.display()
    )
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
    strip_target_dev_dependencies(root, &mut changed);
    write_sanitized_manifest(manifest_path, changed, value)
}

fn strip_target_dev_dependencies(
    root: &mut toml::map::Map<String, toml::Value>,
    changed: &mut bool,
) {
    let Some(target) = root
        .get_mut("target")
        .and_then(|value| value.as_table_mut())
    else {
        return;
    };

    for target_entry in target.values_mut() {
        let Some(target_table) = target_entry.as_table_mut() else {
            continue;
        };
        if target_table.remove("dev-dependencies").is_some() {
            *changed = true;
        }
    }
}

fn write_sanitized_manifest(manifest_path: &Path, changed: bool, value: toml::Value) -> Result<()> {
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

#[cfg(unix)]
pub(super) async fn set_executable_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = tokio::fs::metadata(path).await?.permissions();
    permissions.set_mode(0o755);
    tokio::fs::set_permissions(path, permissions).await?;
    Ok(())
}

#[cfg(not(unix))]
pub(super) async fn set_executable_permissions(_path: &Path) -> Result<()> {
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
