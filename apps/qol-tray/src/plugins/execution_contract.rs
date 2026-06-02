use super::PluginManifest;
use crate::plugins::PluginSource;
use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub(crate) struct MissingBinaryContractError {
    plugin_id: String,
    plugin_path: PathBuf,
    command_field: &'static str,
    command: String,
}

impl std::fmt::Display for MissingBinaryContractError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {:?} binary not found for plugin {} in {:?}",
            self.command_field, self.command, self.plugin_id, self.plugin_path
        )
    }
}

impl std::error::Error for MissingBinaryContractError {}

#[cfg(test)]
pub(crate) fn resolve_plugin_command_path(plugin_dir: &Path, command: &str) -> Option<PathBuf> {
    resolve_plugin_command_path_for_source(plugin_dir, command, None)
}

pub(crate) fn resolve_plugin_command_path_for_source(
    plugin_dir: &Path,
    command: &str,
    source: Option<&PluginSource>,
) -> Option<PathBuf> {
    let command_path = Path::new(command);
    if command_path.components().count() != 1 {
        return None;
    }

    let canonical_plugin_dir = std::fs::canonicalize(plugin_dir).ok()?;
    resolve_candidate_by_source(source, plugin_dir, command_path, &canonical_plugin_dir)
        .or_else(|| resolve_windows_candidate(plugin_dir, command_path, &canonical_plugin_dir))
}

pub(crate) fn validate_execution_contract(
    plugin_id: &str,
    manifest: &PluginManifest,
    plugin_path: &Path,
) -> Result<()> {
    validate_execution_contract_for_source(plugin_id, manifest, plugin_path, None)
}

pub(crate) fn validate_execution_contract_for_source(
    plugin_id: &str,
    manifest: &PluginManifest,
    plugin_path: &Path,
    source: Option<&PluginSource>,
) -> Result<()> {
    ensure_command_binary_exists(
        plugin_id,
        plugin_path,
        "runtime.command",
        source,
        manifest
            .runtime
            .as_ref()
            .map(|runtime| runtime.command.as_str()),
    )?;
    ensure_command_binary_exists(
        plugin_id,
        plugin_path,
        "daemon.command",
        source,
        manifest
            .daemon
            .as_ref()
            .filter(|daemon| daemon.enabled)
            .map(|daemon| daemon.command.as_str()),
    )?;
    Ok(())
}

fn resolve_candidate_by_source(
    source: Option<&PluginSource>,
    plugin_dir: &Path,
    command_path: &Path,
    canonical_plugin_dir: &Path,
) -> Option<PathBuf> {
    if source.is_some_and(PluginSource::is_live_source) {
        return resolve_dev_candidate(plugin_dir, command_path, canonical_plugin_dir)
            .or_else(|| resolve_primary_candidate(plugin_dir, command_path, canonical_plugin_dir));
    }

    resolve_primary_candidate(plugin_dir, command_path, canonical_plugin_dir)
        .or_else(|| resolve_dev_candidate(plugin_dir, command_path, canonical_plugin_dir))
}

fn resolve_primary_candidate(
    plugin_dir: &Path,
    command_path: &Path,
    canonical_plugin_dir: &Path,
) -> Option<PathBuf> {
    let primary = plugin_dir.join(command_path.as_os_str());
    if is_allowed_candidate(&primary, canonical_plugin_dir) {
        return Some(primary);
    }

    None
}

fn resolve_dev_candidate(
    plugin_dir: &Path,
    command_path: &Path,
    canonical_plugin_dir: &Path,
) -> Option<PathBuf> {
    #[cfg(feature = "dev")]
    {
        for profile in ["debug", "release"] {
            let candidate = plugin_dir.join("target").join(profile).join(command_path);
            if is_allowed_candidate(&candidate, canonical_plugin_dir) {
                return Some(candidate);
            }
        }

        // Cargo workspace (monorepo): every member builds into the one
        // shared target dir at the workspace root, not the plugin's own
        // folder. Trust that target dir as the allowed root.
        if let Some(workspace_target) = workspace_target_dir(plugin_dir) {
            for profile in ["debug", "release"] {
                let candidate = workspace_target.join(profile).join(command_path);
                if is_allowed_candidate(&candidate, &workspace_target) {
                    return Some(candidate);
                }
            }
        }
    }

    #[cfg(not(feature = "dev"))]
    {
        let _ = plugin_dir;
        let _ = command_path;
        let _ = canonical_plugin_dir;
    }

    None
}

/// Walk up from a plugin dir to the cargo workspace root (the first
/// ancestor whose `Cargo.toml` declares `[workspace]`) and return its
/// canonical `target` dir. `None` when the plugin is not inside a
/// workspace (the classic one-repo-per-plugin layout).
#[cfg(feature = "dev")]
fn workspace_target_dir(plugin_dir: &Path) -> Option<PathBuf> {
    let mut dir = std::fs::canonicalize(plugin_dir).ok()?;
    loop {
        let manifest = dir.join("Cargo.toml");
        if manifest.is_file() {
            let is_workspace = std::fs::read_to_string(&manifest)
                .ok()
                .and_then(|text| toml::from_str::<toml::Value>(&text).ok())
                .is_some_and(|value| value.get("workspace").is_some());
            if is_workspace {
                return std::fs::canonicalize(dir.join("target")).ok();
            }
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn resolve_windows_candidate(
    plugin_dir: &Path,
    command_path: &Path,
    canonical_plugin_dir: &Path,
) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let primary = plugin_dir.join(command_path.as_os_str());
        if primary.extension().is_none() {
            let exe_candidate = primary.with_extension("exe");
            if is_allowed_candidate(&exe_candidate, canonical_plugin_dir) {
                return Some(exe_candidate);
            }
        }
    }

    #[cfg(not(windows))]
    {
        let _ = plugin_dir;
        let _ = command_path;
        let _ = canonical_plugin_dir;
    }

    None
}

fn is_allowed_candidate(path: &Path, canonical_plugin_dir: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    std::fs::canonicalize(path)
        .ok()
        .is_some_and(|resolved| resolved.starts_with(canonical_plugin_dir))
}

fn ensure_command_binary_exists(
    plugin_id: &str,
    plugin_path: &Path,
    command_field: &'static str,
    source: Option<&PluginSource>,
    command: Option<&str>,
) -> Result<()> {
    let Some(command) = command else {
        return Ok(());
    };
    if resolve_plugin_command_path_for_source(plugin_path, command, source).is_some() {
        return Ok(());
    }

    Err(MissingBinaryContractError {
        plugin_id: plugin_id.to_string(),
        plugin_path: plugin_path.to_path_buf(),
        command_field,
        command: command.to_string(),
    }
    .into())
}
