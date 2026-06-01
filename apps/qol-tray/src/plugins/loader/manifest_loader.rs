use crate::plugins::resolver::ResolvedPlugin;
use crate::plugins::{Plugin, PluginId, PluginManifest, PluginSource};
use anyhow::{Context, Result};
use std::path::Path;

pub(super) fn load_plugin_with_id(id: &str, path: &Path) -> Result<Plugin> {
    load_plugin_with_source(PluginId::new(id), path, PluginSource::Installed)
}

pub(super) fn load_resolved_plugin(resolved: &ResolvedPlugin) -> Result<Plugin> {
    load_plugin_with_source(resolved.id.clone(), &resolved.path, resolved.source.clone())
}

fn load_plugin_with_source(id: PluginId, path: &Path, source: PluginSource) -> Result<Plugin> {
    let manifest_path = manifest_path(path);
    ensure_manifest_exists(path, &manifest_path)?;
    let manifest_content = read_manifest(&manifest_path)?;
    let manifest = parse_manifest(&manifest_content)?;
    validate_manifest_contract(id.as_str(), &manifest, path, &source)?;
    Ok(Plugin::new_with_source(
        id,
        manifest,
        path.to_path_buf(),
        source,
    ))
}

fn manifest_path(path: &Path) -> std::path::PathBuf {
    path.join("plugin.toml")
}

fn ensure_manifest_exists(path: &Path, manifest_path: &Path) -> Result<()> {
    if manifest_path.exists() {
        return Ok(());
    }

    anyhow::bail!("No plugin.toml found in {:?}", path)
}

fn read_manifest(manifest_path: &Path) -> Result<String> {
    std::fs::read_to_string(manifest_path).context("Failed to read plugin.toml")
}

fn parse_manifest(manifest_content: &str) -> Result<PluginManifest> {
    let manifest: PluginManifest =
        toml::from_str(manifest_content).context("Failed to parse plugin.toml")?;
    manifest
        .validate()
        .context("Invalid plugin.toml contract")?;
    Ok(manifest)
}

fn validate_manifest_contract(
    id: &str,
    manifest: &PluginManifest,
    path: &Path,
    source: &PluginSource,
) -> Result<()> {
    if !manifest.plugin.supports_current_platform() {
        return Ok(());
    }

    crate::plugins::validate_execution_contract_for_source(id, manifest, path, Some(source))
}
