use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
pub struct LinkedPlugin {
    pub id: String,
    pub name: String,
    pub is_symlink: bool,
    pub target: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinkRequest {
    pub path: String,
}

pub fn list_linked_plugins(plugins_dir: &Path) -> Result<Vec<LinkedPlugin>, String> {
    if !plugins_dir.exists() {
        return Ok(vec![]);
    }

    let entries = std::fs::read_dir(plugins_dir)
        .map_err(|e| format!("Failed to read plugins dir: {}", e))?;

    let mut plugins = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "backup") {
            continue;
        }

        let id = entry.file_name().to_string_lossy().to_string();

        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };

        let is_symlink = metadata.file_type().is_symlink();
        let target = if is_symlink {
            std::fs::read_link(&path)
                .ok()
                .map(|p| p.to_string_lossy().to_string())
        } else {
            None
        };

        let name = std::fs::read_to_string(path.join("plugin.toml"))
            .ok()
            .and_then(|s| toml::from_str::<crate::plugins::PluginManifest>(&s).ok())
            .map(|m| m.plugin.name)
            .unwrap_or_else(|| id.clone());

        plugins.push(LinkedPlugin {
            id,
            name,
            is_symlink,
            target,
        });
    }

    plugins.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(plugins)
}

pub fn create_link(source: &Path, plugins_dir: &Path) -> Result<String, String> {
    if !source.exists() {
        return Err("Source path does not exist".to_string());
    }

    if !source.join("plugin.toml").exists() {
        return Err("No plugin.toml found in source".to_string());
    }

    let plugin_id = source
        .file_name()
        .ok_or("Invalid path")?
        .to_string_lossy()
        .to_string();

    let link_path = plugins_dir.join(&plugin_id);

    backup_existing_if_not_symlink(&link_path)?;
    create_symlink(source, &link_path)
        .map_err(|e| format!("Failed to create symlink: {}", e))?;

    log::info!("Created plugin link: {} -> {:?}", plugin_id, source);
    Ok(plugin_id)
}

pub fn remove_link(id: &str, plugins_dir: &Path) -> Result<(), String> {
    let link_path = plugins_dir.join(id);

    remove_symlink(&link_path)?;

    if let Err(e) = restore_from_backup(&link_path) {
        log::warn!("No backup to restore for {}: {}", id, e);
    }

    log::info!("Unlinked plugin: {}", id);
    Ok(())
}

fn backup_existing_if_not_symlink(path: &Path) -> Result<(), String> {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return Ok(());
    };

    if metadata.file_type().is_symlink() {
        return Err("Already linked".to_string());
    }

    let backup_path = path.with_extension("backup");
    if backup_path.exists() {
        std::fs::remove_dir_all(&backup_path)
            .map_err(|e| format!("Failed to remove old backup: {}", e))?;
    }

    std::fs::rename(path, &backup_path).map_err(|e| format!("Failed to backup existing: {}", e))
}

fn create_symlink(source: &Path, link: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(source, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(source, link)
    }
}

fn remove_symlink(path: &Path) -> Result<(), String> {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return Err("Plugin not found".to_string());
    };

    if !metadata.file_type().is_symlink() {
        return Err("Not a symlink - use uninstall instead".to_string());
    }

    std::fs::remove_file(path).map_err(|e| format!("Failed to remove link: {}", e))
}

fn restore_from_backup(path: &Path) -> Result<(), String> {
    let backup_path = path.with_extension("backup");
    if !backup_path.exists() {
        return Err("No backup exists".to_string());
    }

    std::fs::rename(&backup_path, path).map_err(|e| format!("Failed to restore backup: {}", e))
}
