use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub fn load_dev_links(config_dir: &Path) -> HashMap<String, PathBuf> {
    let path = dev_links_path(config_dir);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return HashMap::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

pub fn create_link(source: &Path, config_dir: &Path) -> Result<String, String> {
    let plugin_id = validate_link_source(source)?;
    let mut links = load_dev_links(config_dir);

    if links.contains_key(&plugin_id) {
        return Err("Already linked".to_string());
    }

    links.insert(plugin_id.clone(), source.to_path_buf());
    save_dev_links(config_dir, &links)?;
    log::info!("Created dev-link: {} -> {:?}", plugin_id, source);
    Ok(plugin_id)
}

pub fn remove_link(id: &str, config_dir: &Path) -> Result<(), String> {
    let mut links = load_dev_links(config_dir);

    if links.remove(id).is_none() {
        return Err("Plugin not dev-linked".to_string());
    }

    save_dev_links(config_dir, &links)?;
    log::info!("Removed dev-link: {}", id);
    Ok(())
}

fn validate_link_source(source: &Path) -> Result<String, String> {
    if !source.exists() {
        return Err("Source path does not exist".to_string());
    }

    if !source.join("plugin.toml").exists() {
        return Err("No plugin.toml found in source".to_string());
    }

    let plugin_id = source
        .file_name()
        .ok_or_else(|| "Invalid path".to_string())?
        .to_string_lossy()
        .to_string();

    Ok(plugin_id)
}

fn save_dev_links(config_dir: &Path, links: &HashMap<String, PathBuf>) -> Result<(), String> {
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
    config_dir.join("dev-links.json")
}

fn temp_dev_links_path(config_dir: &Path) -> PathBuf {
    config_dir.join(".dev-links.json.tmp")
}
