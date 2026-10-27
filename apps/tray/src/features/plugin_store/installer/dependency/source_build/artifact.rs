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

pub(super) async fn stage_built_binary(source_path: &Path, destination: &Path) -> Result<()> {
    let _ = tokio::fs::remove_file(destination).await;

    tokio::fs::copy(source_path, destination)
        .await
        .with_context(|| stage_copy_error(source_path, destination))?;

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

pub(super) async fn set_executable_permissions(path: &Path) -> Result<()> {
    let metadata = tokio::fs::metadata(path).await?;
    let Some(permissions) = super::super::super::super::platform::executable_permissions(metadata)
    else {
        return Ok(());
    };
    tokio::fs::set_permissions(path, permissions).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_built_binary_stages_to_the_destination_it_is_given() {
        let dir = tempfile::tempdir().unwrap();
        let built = dir.path().join("target-release-binary");
        let destination = dir.path().join("qol-alt-tab.new");
        std::fs::write(&built, b"built bytes").unwrap();

        stage_built_binary(&built, &destination).await.unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"built bytes");
        assert!(!dir.path().join("qol-alt-tab").exists());
    }

    #[tokio::test]
    async fn staging_replaces_a_stale_destination() {
        let dir = tempfile::tempdir().unwrap();
        let built = dir.path().join("target-release-binary");
        let destination = dir.path().join("qol-alt-tab.new");
        std::fs::write(&built, b"built bytes").unwrap();
        std::fs::write(&destination, b"stale bytes").unwrap();

        stage_built_binary(&built, &destination).await.unwrap();

        assert_eq!(std::fs::read(&destination).unwrap(), b"built bytes");
    }
}
