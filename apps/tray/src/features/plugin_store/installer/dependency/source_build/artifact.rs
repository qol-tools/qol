use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub(super) fn dependency_binary_output_path(plugin_dir: &Path, binary_name: &str) -> PathBuf {
    super::super::super::super::platform::dependency_binary_output_path(plugin_dir, binary_name)
}

pub(super) fn built_binary_path(plugin_dir: &Path, binary_name: &str) -> Result<PathBuf> {
    built_binary_candidates(plugin_dir, binary_name)
        .into_iter()
        .find(|path| path.is_file())
        .with_context(|| missing_built_binary(binary_name, plugin_dir))
}

pub(super) async fn install_built_binary(source_path: &Path, output_path: &Path) -> Result<()> {
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

fn missing_built_binary(binary_name: &str, plugin_dir: &Path) -> String {
    format!(
        "Built binary not found for {} in {}",
        binary_name,
        plugin_dir.join("target").join("release").display()
    )
}

fn built_binary_candidates(plugin_dir: &Path, binary_name: &str) -> Vec<PathBuf> {
    super::super::super::super::platform::built_binary_candidates(plugin_dir, binary_name)
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

pub(super) async fn set_executable_permissions(path: &Path) -> Result<()> {
    let metadata = tokio::fs::metadata(path).await?;
    let Some(permissions) = super::super::super::super::platform::executable_permissions(metadata)
    else {
        return Ok(());
    };
    tokio::fs::set_permissions(path, permissions).await?;
    Ok(())
}
