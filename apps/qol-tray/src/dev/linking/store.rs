use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn load_dev_links(config_dir: &Path) -> HashMap<String, PathBuf> {
    let path = dev_links_path(config_dir);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    let links: HashMap<String, PathBuf> = serde_json::from_str(&content).unwrap_or_default();
    canonicalize_stale_worktree_paths(config_dir, links)
}

fn canonicalize_stale_worktree_paths(
    config_dir: &Path,
    mut links: HashMap<String, PathBuf>,
) -> HashMap<String, PathBuf> {
    let mut changed = false;
    for (id, path) in links.iter_mut() {
        let Some(canonical) = crate::dev::find_git_worktree_base(path) else {
            continue;
        };
        if canonical != *path {
            log::info!("Auto-healed dev-link {}: {:?} -> {:?}", id, path, canonical);
            *path = canonical;
            changed = true;
        }
    }
    if changed {
        let _ = save_dev_links(config_dir, &links);
    }
    links
}

pub fn set_active_worktree_branch(config_dir: &Path, branch: Option<&str>) -> Result<(), String> {
    let path = config_dir.join("dev/active-worktree.txt");
    if let Some(branch) = branch {
        std::fs::create_dir_all(config_dir.join("dev"))
            .map_err(|e| format!("Failed to create dev directory: {}", e))?;
        return std::fs::write(&path, branch.trim()).map_err(|e| format!("Failed to write: {}", e));
    }

    let _ = std::fs::remove_file(&path);
    Ok(())
}

pub fn get_active_worktree_branch(config_dir: &Path) -> Option<String> {
    let path = config_dir.join("dev/active-worktree.txt");
    std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn create_link(source: &Path, config_dir: &Path) -> Result<String, String> {
    let plugin_id = validate_link_source(source)?;

    if crate::plugins::registry::dev_linked_paths(config_dir).contains_key(&plugin_id) {
        return Err("Already linked".to_string());
    }

    let canonical =
        crate::dev::find_git_worktree_base(source).unwrap_or_else(|| source.to_path_buf());
    let mut links = load_dev_links(config_dir);
    links.insert(plugin_id.clone(), canonical.clone());
    save_dev_links(config_dir, &links)?;
    crate::plugins::registry::record_dev_link_create(config_dir, &plugin_id, canonical.clone())?;
    log::info!("Created dev-link: {} -> {:?}", plugin_id, canonical);
    Ok(plugin_id)
}

pub fn remove_link(id: &str, config_dir: &Path) -> Result<(), String> {
    if !crate::plugins::registry::dev_linked_paths(config_dir).contains_key(id) {
        return Err("Plugin not dev-linked".to_string());
    }
    let mut links = load_dev_links(config_dir);
    links.remove(id);
    save_dev_links(config_dir, &links)?;
    crate::plugins::registry::record_dev_link_remove(config_dir, id)?;
    log::info!("Removed dev-link: {}", id);
    Ok(())
}

fn validate_link_source(source: &Path) -> Result<String, String> {
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

    let plugin_id = canonical
        .file_name()
        .ok_or_else(|| "Invalid path".to_string())?
        .to_string_lossy()
        .to_string();

    Ok(plugin_id)
}

fn save_dev_links(config_dir: &Path, links: &HashMap<String, PathBuf>) -> Result<(), String> {
    let dev_dir = config_dir.join("dev");
    std::fs::create_dir_all(&dev_dir)
        .map_err(|e| format!("Failed to create dev directory: {}", e))?;
    let path = dev_links_path(config_dir);
    let tmp_path = temp_dev_links_path(config_dir);
    let content = serde_json::to_string_pretty(links)
        .map_err(|e| format!("Failed to serialize dev-links: {}", e))?;

    std::fs::write(&tmp_path, &content)
        .map_err(|e| format!("Failed to write dev-links temp file: {}", e))?;
    std::fs::rename(&tmp_path, &path)
        .map_err(|e| format!("Failed to finalize dev-links.json: {}", e))
}

fn dev_links_path(config_dir: &Path) -> PathBuf {
    config_dir.join("dev/links.json")
}

fn temp_dev_links_path(config_dir: &Path) -> PathBuf {
    config_dir.join("dev/.links.json.tmp")
}
