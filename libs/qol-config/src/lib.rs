pub mod contract;
pub mod normalized;
pub mod validation;

use serde::de::DeserializeOwned;
use std::fs;
use std::path::{Path, PathBuf};

pub fn base_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .map(|path| path.join("qol-tray"))
}

pub fn config_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let base = match base_data_dir() {
        Some(base) => base,
        None => return roots,
    };
    if let Some(id) = install_id_from_env() {
        roots.push(base.join("installs").join(id));
    }
    if let Some(id) = install_id_from_active_file(&base) {
        let candidate = base.join("installs").join(id);
        if !roots.contains(&candidate) {
            roots.push(candidate);
        }
    }
    if !roots.contains(&base) {
        roots.push(base);
    }
    if let Some(config_dir) = dirs::config_dir().map(|p| p.join("qol-tray")) {
        if !roots.contains(&config_dir) {
            roots.push(config_dir);
        }
    }
    roots
}

pub fn plugin_config_paths(names: &[&str]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for root in config_roots() {
        for name in names {
            let candidate = root.join("plugins").join(name).join("config.json");
            if !paths.contains(&candidate) {
                paths.push(candidate);
            }
        }
    }
    paths
}

pub fn load_plugin_config<T: DeserializeOwned + Default>(names: &[&str]) -> T {
    for path in plugin_config_paths(names) {
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(_) => continue,
        };
        match serde_json::from_str::<T>(&contents) {
            Ok(config) => {
                eprintln!("[config] loaded from {}", path.display());
                return config;
            }
            Err(e) => {
                eprintln!("[config] failed to parse {}: {}", path.display(), e);
            }
        }
    }
    T::default()
}

pub fn install_id_from_env() -> Option<String> {
    let value = std::env::var("QOL_TRAY_INSTALL_ID").ok()?;
    let trimmed = value.trim();
    if !valid_install_id(trimmed) {
        return None;
    }
    Some(trimmed.to_string())
}

pub fn install_id_from_active_file(base_data_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(base_data_dir.join("active-install-id")).ok()?;
    let trimmed = content.trim();
    if !valid_install_id(trimmed) {
        return None;
    }
    Some(trimmed.to_string())
}

pub fn valid_install_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
