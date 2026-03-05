use super::PluginManifest;
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

pub(crate) fn resolve_plugin_command_path(plugin_dir: &Path, command: &str) -> Option<PathBuf> {
    let command_path = Path::new(command);
    if command_path.components().count() != 1 {
        return None;
    }

    let canonical_plugin_dir = std::fs::canonicalize(plugin_dir).ok()?;
    resolve_primary_candidate(plugin_dir, command_path, &canonical_plugin_dir)
        .or_else(|| resolve_dev_candidate(plugin_dir, command_path, &canonical_plugin_dir))
        .or_else(|| resolve_windows_candidate(plugin_dir, command_path, &canonical_plugin_dir))
}

pub(crate) fn validate_execution_contract(
    plugin_id: &str,
    manifest: &PluginManifest,
    plugin_path: &Path,
) -> Result<()> {
    ensure_command_binary_exists(
        plugin_id,
        plugin_path,
        "runtime.command",
        manifest
            .runtime
            .as_ref()
            .map(|runtime| runtime.command.as_str()),
    )?;
    ensure_command_binary_exists(
        plugin_id,
        plugin_path,
        "daemon.command",
        manifest
            .daemon
            .as_ref()
            .filter(|daemon| daemon.enabled)
            .map(|daemon| daemon.command.as_str()),
    )?;
    Ok(())
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
        let debug_target = plugin_dir
            .join("target")
            .join("debug")
            .join(command_path.as_os_str());
        if is_allowed_candidate(&debug_target, canonical_plugin_dir) {
            return Some(debug_target);
        }

        let release_target = plugin_dir
            .join("target")
            .join("release")
            .join(command_path.as_os_str());
        if is_allowed_candidate(&release_target, canonical_plugin_dir) {
            return Some(release_target);
        }
    }

    None
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
    command: Option<&str>,
) -> Result<()> {
    let Some(command) = command else {
        return Ok(());
    };
    if resolve_plugin_command_path(plugin_path, command).is_some() {
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
