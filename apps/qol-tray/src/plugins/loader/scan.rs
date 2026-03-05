#[cfg(test)]
use super::manifest_loader;
#[cfg(test)]
use crate::plugins::execution_contract::MissingBinaryContractError;
#[cfg(test)]
use crate::plugins::Plugin;
#[cfg(test)]
use anyhow::Context;
#[cfg(test)]
use anyhow::Result;
#[cfg(test)]
use std::path::{Path, PathBuf};

#[cfg(test)]
pub(super) fn discover_plugin_items(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    if !dir.exists() {
        log::warn!("Plugin directory does not exist: {:?}", dir);
        return Ok(Vec::new());
    }

    let items = std::fs::read_dir(dir)
        .context("Failed to read plugins directory")?
        .filter_map(|entry| entry.ok())
        .filter_map(read_plugin_item)
        .collect();

    Ok(items)
}

#[cfg(test)]
pub(super) fn load_items(items: &[(String, PathBuf)]) -> Result<Vec<Plugin>> {
    let mut diagnostics = LoadDiagnostics::new(items.len());
    let mut plugins = Vec::new();

    for (id, path) in items {
        process_item(id, path, &mut plugins, &mut diagnostics);
    }

    diagnostics.log();
    Ok(plugins)
}

#[cfg(test)]
fn read_plugin_item(entry: std::fs::DirEntry) -> Option<(String, PathBuf)> {
    let path = entry.path();
    if skip_plugin_path(&path) {
        return None;
    }
    let id = entry.file_name().into_string().ok()?;
    Some((id, path))
}

#[cfg(test)]
fn skip_plugin_path(path: &Path) -> bool {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return true;
    };
    metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || path.extension().is_some_and(|ext| ext == "backup")
}

#[cfg(test)]
fn process_item(
    id: &str,
    path: &Path,
    plugins: &mut Vec<Plugin>,
    diagnostics: &mut LoadDiagnostics,
) {
    match manifest_loader::load_plugin_with_id(id, path) {
        Ok(plugin) => push_plugin(plugin, plugins, diagnostics),
        Err(error) => diagnostics.record_error(id, path, error),
    }
}

#[cfg(test)]
fn push_plugin(plugin: Plugin, plugins: &mut Vec<Plugin>, diagnostics: &mut LoadDiagnostics) {
    if !plugin.manifest.plugin.supports_current_platform() {
        diagnostics.record_skipped_platform(&plugin);
        return;
    }

    log::info!(
        "Loaded plugin: {} ({})",
        plugin.manifest.plugin.name,
        plugin.id
    );
    plugins.push(plugin);
}

#[cfg(test)]
struct LoadDiagnostics {
    discovered: usize,
    skipped_platform: usize,
    invalid_manifest: usize,
    missing_binaries: usize,
}

#[cfg(test)]
impl LoadDiagnostics {
    fn new(discovered: usize) -> Self {
        Self {
            discovered,
            skipped_platform: 0,
            invalid_manifest: 0,
            missing_binaries: 0,
        }
    }

    fn record_skipped_platform(&mut self, plugin: &Plugin) {
        self.skipped_platform += 1;
        log::info!(
            "Skipping plugin {} (unsupported platform: {})",
            plugin.id,
            std::env::consts::OS
        );
    }

    fn record_error(&mut self, id: &str, path: &Path, error: anyhow::Error) {
        if let Some(missing) = error.downcast_ref::<MissingBinaryContractError>() {
            self.missing_binaries += 1;
            log::warn!("Skipping plugin {} (missing binary): {}", id, missing);
            return;
        }

        self.invalid_manifest += 1;
        log::warn!("Failed to load plugin from {:?}: {:#}", path, error);
    }

    fn log(&self) {
        log::info!(
            "Plugin diagnostics: discovered={}, loaded={}, unsupported_platform={}, invalid={}, missing_binaries={}",
            self.discovered,
            self.loaded(),
            self.skipped_platform,
            self.invalid_manifest,
            self.missing_binaries
        );
    }

    fn loaded(&self) -> usize {
        self.discovered
            .saturating_sub(self.skipped_platform + self.invalid_manifest + self.missing_binaries)
    }
}
