use anyhow::Result;
use std::path::{Path, PathBuf};

const CONFIG_FILE_NAME: &str = "config.json";
const MANIFEST_FILE_NAME: &str = "plugin.toml";
const CONFIG_CONTRACT_FILE_NAME: &str = "qol-config.toml";
const RUNABLE_CONTRACT_FILE_NAME: &str = "qol-runtime.toml";

pub(crate) fn resolve_plugin_root(plugin_id: &str) -> Result<PathBuf> {
    if !crate::paths::is_safe_path_component(plugin_id) {
        anyhow::bail!("Invalid plugin ID: {}", plugin_id);
    }
    let plugins_dir = crate::paths::plugins_dir()?;
    Ok(resolve_plugin_root_from_plugins_dir(
        &plugins_dir,
        plugin_id,
    ))
}

pub(crate) fn resolve_plugin_root_from_plugins_dir(plugins_dir: &Path, plugin_id: &str) -> PathBuf {
    if let Some(active) = crate::plugins::registry::current_active_path(plugin_id) {
        return dev_resolved_root(plugins_dir, plugin_id, &active).unwrap_or(active);
    }
    plugins_dir.join(plugin_id)
}

#[cfg(feature = "dev")]
fn dev_resolved_root(plugins_dir: &Path, plugin_id: &str, active: &Path) -> Option<PathBuf> {
    let branch = crate::dev::get_active_worktree_branch(&crate::paths::shared_config_dir().ok()?)?;
    let mut map = std::collections::HashMap::new();
    map.insert(plugin_id.to_string(), active.to_path_buf());
    let resolved = crate::dev::resolve_worktree_paths(&map, Some(&branch));
    Some(match resolved.get(plugin_id) {
        Some(root) => root.clone(),
        None => plugins_dir.join(plugin_id),
    })
}

#[cfg(not(feature = "dev"))]
fn dev_resolved_root(_plugins_dir: &Path, _plugin_id: &str, _active: &Path) -> Option<PathBuf> {
    None
}

pub(crate) fn config_path(plugin_root: &Path) -> PathBuf {
    plugin_root.join(CONFIG_FILE_NAME)
}

pub(crate) fn config_contract_path(plugin_root: &Path) -> PathBuf {
    plugin_root.join(CONFIG_CONTRACT_FILE_NAME)
}

pub(crate) fn runable_contract_path(plugin_root: &Path) -> PathBuf {
    plugin_root.join(RUNABLE_CONTRACT_FILE_NAME)
}

pub(crate) fn has_config(plugin_root: &Path) -> bool {
    config_contract_path(plugin_root).exists()
}

pub(crate) fn has_manifest(plugin_root: &Path) -> bool {
    plugin_root.join(MANIFEST_FILE_NAME).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runable_contract_path_joins_filename() {
        let root = Path::new("/tmp/plugin-foo");
        assert_eq!(
            runable_contract_path(root),
            Path::new("/tmp/plugin-foo/qol-runtime.toml")
        );
    }
}
