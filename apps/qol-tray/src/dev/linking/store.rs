use std::path::{Path, PathBuf};

pub(crate) fn set_active_worktree_branch(
    config_dir: &Path,
    branch: Option<&str>,
) -> Result<(), String> {
    qol_dev_build::tray::set_active_worktree_marker(config_dir, branch)
}

pub fn get_active_worktree_branch(config_dir: &Path) -> Option<String> {
    qol_dev_build::tray::read_active_worktree_marker(config_dir)
}

pub fn create_link(source: &Path, config_dir: &Path) -> Result<String, String> {
    let (plugin_id, plugin_root) = validate_link_source(source)?;

    if crate::plugins::registry::dev_linked_paths(config_dir).contains_key(&plugin_id) {
        return Err("Already linked".to_string());
    }

    crate::plugins::registry::record_dev_link_create(config_dir, &plugin_id, plugin_root.clone())?;
    log::info!("Created dev-link: {} -> {:?}", plugin_id, plugin_root);
    Ok(plugin_id)
}

pub fn remove_link(id: &str, config_dir: &Path) -> Result<(), String> {
    if !crate::plugins::registry::dev_linked_paths(config_dir).contains_key(id) {
        return Err("Plugin not dev-linked".to_string());
    }
    crate::plugins::registry::record_dev_link_remove(config_dir, id)?;
    log::info!("Removed dev-link: {}", id);
    Ok(())
}

fn validate_link_source(source: &Path) -> Result<(String, PathBuf), String> {
    if !source.exists() {
        return Err("Source path does not exist".to_string());
    }

    let canonical = source
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize {}: {}", source.display(), e))?;

    if !canonical.is_dir() {
        return Err(format!("Not a directory: {}", canonical.display()));
    }

    let manifest_path = canonical.join("plugin.toml");
    if !manifest_path.exists() {
        return Err("No plugin.toml found in source".to_string());
    }

    let manifest_content = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("Failed to read {}: {}", manifest_path.display(), e))?;
    let manifest: crate::plugins::manifest::PluginManifest = toml::from_str(&manifest_content)
        .map_err(|e| format!("Failed to parse {}: {}", manifest_path.display(), e))?;
    manifest.validate().map_err(|e| {
        format!(
            "Manifest validation failed for {}: {}",
            manifest_path.display(),
            e
        )
    })?;

    let plugin_id = manifest
        .plugin
        .require_declared_id()
        .map_err(|e| {
            format!(
                "Manifest validation failed for {}: {}",
                manifest_path.display(),
                e
            )
        })?
        .as_str()
        .to_string();

    Ok((plugin_id, canonical))
}
