use anyhow::{Context, Result};
use std::path::Path;

use super::staging::{cleanup_temp_dir, swap_plugin_dirs, update_backup_dir, update_staging_dir};

pub(super) fn plugin_id(plugin_root: &Path) -> Result<String> {
    let manifest = crate::plugins::manifest::PluginManifest::load_and_validate(
        plugin_root.join("plugin.toml"),
    )?;
    Ok(manifest.plugin.require_declared_id()?.as_str().to_string())
}

pub(super) async fn install(
    plugins_dir: &Path,
    workspace_root: &Path,
    plugin_root: &Path,
    plugin_id: &str,
) -> Result<()> {
    let manifest = crate::plugins::manifest::PluginManifest::load_and_validate(
        plugin_root.join("plugin.toml"),
    )?;
    let binaries = dependency_names(&manifest, plugin_id)?;
    let excluded_root_files = excluded_root_files(plugin_root, &binaries);
    let staging_dir = update_staging_dir(plugins_dir, plugin_id);
    let backup_dir = update_backup_dir(plugins_dir, plugin_id);
    let target_dir = plugins_dir.join(plugin_id);
    cleanup_temp_dir(&staging_dir).await;
    cleanup_temp_dir(&backup_dir).await;

    let prepared = prepare_staging(
        workspace_root,
        plugin_root,
        &staging_dir,
        &binaries,
        &excluded_root_files,
    )
    .await;
    if let Err(error) = prepared {
        cleanup_temp_dir(&staging_dir).await;
        return Err(error);
    }

    let source = crate::plugins::PluginSource::Installed;
    let validated = crate::plugins::validate_execution_contract_for_source(
        plugin_id,
        &manifest,
        &staging_dir,
        Some(&source),
    );
    if let Err(error) = validated {
        cleanup_temp_dir(&staging_dir).await;
        return Err(error);
    }
    let promoted = promote(&target_dir, &staging_dir, &backup_dir).await;
    if promoted.is_err() {
        cleanup_temp_dir(&staging_dir).await;
    }
    promoted
}

fn dependency_names(
    manifest: &crate::plugins::manifest::PluginManifest,
    plugin_id: &str,
) -> Result<Vec<String>> {
    let binaries = manifest
        .dependencies
        .as_ref()
        .map(|dependencies| {
            dependencies
                .binaries
                .iter()
                .map(|binary| binary.name.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if binaries.is_empty() {
        anyhow::bail!("{plugin_id} declares no installable binaries");
    }
    Ok(binaries)
}

fn excluded_root_files(plugin_root: &Path, binaries: &[String]) -> Vec<String> {
    binaries
        .iter()
        .filter_map(|binary| {
            super::super::platform::dependency_binary_output_path(plugin_root, binary)
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .collect()
}

async fn prepare_staging(
    workspace_root: &Path,
    plugin_root: &Path,
    staging_dir: &Path,
    binaries: &[String],
    excluded_root_files: &[String],
) -> Result<()> {
    tokio::fs::create_dir_all(staging_dir)
        .await
        .with_context(|| format!("failed to create {}", staging_dir.display()))?;
    let excluded = excluded_root_files
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    for file in qol_workspace::plugin_delivery_files(plugin_root, &excluded)? {
        copy_file(&file.source, &staging_dir.join(file.relative_path)).await?;
    }
    for binary in binaries {
        install_binary(workspace_root, staging_dir, binary).await?;
    }
    Ok(())
}

async fn install_binary(
    workspace_root: &Path,
    staging_dir: &Path,
    binary_name: &str,
) -> Result<()> {
    let source = super::super::platform::built_binary_candidates(workspace_root, binary_name)
        .into_iter()
        .find(|path| path.is_file())
        .with_context(|| {
            format!(
                "built release binary {binary_name} is missing under {}",
                workspace_root.join("target").join("release").display()
            )
        })?;
    let destination =
        super::super::platform::dependency_binary_output_path(staging_dir, binary_name);
    copy_file(&source, &destination).await?;
    super::dependency::set_executable_permissions(&destination).await
}

async fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    tokio::fs::copy(source, destination)
        .await
        .with_context(|| {
            format!(
                "failed to copy {} to {}",
                source.display(),
                destination.display()
            )
        })?;
    Ok(())
}

async fn promote(target_dir: &Path, staging_dir: &Path, backup_dir: &Path) -> Result<()> {
    if tokio::fs::metadata(target_dir).await.is_ok() {
        return swap_plugin_dirs(target_dir, staging_dir, backup_dir).await;
    }
    tokio::fs::rename(staging_dir, target_dir)
        .await
        .with_context(|| {
            format!(
                "failed to install local plugin from {} to {}",
                staging_dir.display(),
                target_dir.display()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn local_install_replaces_config_only_directory_with_release_bundle() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let plugin = workspace.join("plugins").join("bluetooth");
        let plugins_dir = root.path().join("installed");
        let target = plugins_dir.join("plugin-bluetooth");
        tokio::fs::create_dir_all(plugin.join("src")).await.unwrap();
        tokio::fs::create_dir_all(workspace.join("target/release"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(&target).await.unwrap();
        tokio::fs::write(
            plugin.join("plugin.toml"),
            r#"[plugin]
id = "plugin-bluetooth"
name = "Bluetooth"
description = ""
version = "1.0.0"

[runtime]
command = "plugin-bluetooth"

[menu]
label = "Bluetooth"
items = []

[[dependencies.binaries]]
name = "plugin-bluetooth"
repo = "qol-tools/plugin-bluetooth"
pattern = "plugin-bluetooth-{os}-{arch}"
"#,
        )
        .await
        .unwrap();
        tokio::fs::write(plugin.join("qol-config.toml"), "schema_version = 1")
            .await
            .unwrap();
        tokio::fs::write(plugin.join("src/main.rs"), "source")
            .await
            .unwrap();
        tokio::fs::write(plugin.join("plugin-bluetooth"), "stale")
            .await
            .unwrap();
        tokio::fs::write(workspace.join("target/release/plugin-bluetooth"), "release")
            .await
            .unwrap();
        tokio::fs::write(target.join("config.json"), "{}")
            .await
            .unwrap();

        install(&plugins_dir, &workspace, &plugin, "plugin-bluetooth")
            .await
            .unwrap();

        assert_eq!(
            tokio::fs::read_to_string(target.join("plugin-bluetooth"))
                .await
                .unwrap(),
            "release"
        );
        assert!(target.join("plugin.toml").is_file());
        assert!(target.join("qol-config.toml").is_file());
        assert!(!target.join("src").exists());
        assert!(!target.join("config.json").exists());
    }
}
