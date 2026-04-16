use std::path::{Path, PathBuf};

pub(crate) fn resolve_plugin_root(plugins_dir: &Path, plugin_id: &str) -> PathBuf {
    if let Some(active) = crate::plugins::registry::current_active_path(plugin_id) {
        return active;
    }
    plugins_dir.join(plugin_id)
}

pub(crate) fn canonical_plugin_root(plugins_dir: &Path, plugin_id: &str) -> Option<PathBuf> {
    let plugin_root = resolve_plugin_root(plugins_dir, plugin_id);
    let canonical_plugin_root = std::fs::canonicalize(plugin_root).ok()?;
    if is_under_installed_root(plugins_dir, &canonical_plugin_root) {
        return Some(canonical_plugin_root);
    }
    if matches_registry_active_path(plugin_id, &canonical_plugin_root) {
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

fn matches_registry_active_path(plugin_id: &str, candidate: &Path) -> bool {
    let Some(active_path) = crate::plugins::registry::current_active_path(plugin_id) else {
        return false;
    };
    let Ok(canonical_active) = std::fs::canonicalize(active_path) else {
        return false;
    };
    candidate == canonical_active
}
