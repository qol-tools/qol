use std::path::{Path, PathBuf};

pub(crate) fn resolve_plugin_root(plugins_dir: &Path, plugin_id: &str) -> PathBuf {
    #[cfg(feature = "dev")]
    if let Some(dev_path) = resolve_dev_link_path(plugin_id) {
        return dev_path;
    }

    plugins_dir.join(plugin_id)
}

pub(crate) fn canonical_plugin_root(plugins_dir: &Path, plugin_id: &str) -> Option<PathBuf> {
    let plugin_root = resolve_plugin_root(plugins_dir, plugin_id);
    let canonical_plugin_root = std::fs::canonicalize(plugin_root).ok()?;
    if is_under_installed_root(plugins_dir, &canonical_plugin_root) {
        return Some(canonical_plugin_root);
    }
    if is_under_dev_link_root(plugin_id, &canonical_plugin_root) {
        return Some(canonical_plugin_root);
    }
    None
}

fn is_under_installed_root(plugins_dir: &Path, candidate: &Path) -> bool {
    let Ok(canonical_plugins_dir) = std::fs::canonicalize(plugins_dir) else {
        return false;
    };
    candidate.starts_with(canonical_plugins_dir)
}

fn is_under_dev_link_root(plugin_id: &str, candidate: &Path) -> bool {
    #[cfg(feature = "dev")]
    {
        let Some(dev_path) = resolve_dev_link_path(plugin_id) else {
            return false;
        };
        let Ok(canonical_dev_path) = std::fs::canonicalize(dev_path) else {
            return false;
        };
        candidate == canonical_dev_path
    }

    #[cfg(not(feature = "dev"))]
    {
        let _ = plugin_id;
        let _ = candidate;
        false
    }
}

#[cfg(feature = "dev")]
fn resolve_dev_link_path(plugin_id: &str) -> Option<PathBuf> {
    let config_dir = crate::paths::shared_config_dir().ok()?;
    let links = crate::dev::load_dev_links(&config_dir);
    links.get(plugin_id).cloned()
}
