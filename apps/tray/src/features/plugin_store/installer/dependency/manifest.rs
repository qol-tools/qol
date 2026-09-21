use anyhow::{Context, Result};
use std::path::Path;

pub(super) async fn load_plugin_manifest(
    plugin_dir: &Path,
) -> Result<crate::plugins::PluginManifest> {
    let manifest_path = plugin_dir.join("plugin.toml");
    ensure_manifest_exists(&manifest_path)?;
    let content = read_manifest(&manifest_path).await?;
    parse_manifest(&content)
}

pub(super) fn validate_execution_contract(
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
