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
    crate::paths::shared_config_dir()
        .ok()
        .and_then(|config_dir| {
            crate::plugins::resolver::resolve_plugin_path(&config_dir, plugin_id)
        })
        .unwrap_or_else(|| plugins_dir.join(plugin_id))
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
