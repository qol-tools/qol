use serde::de::DeserializeOwned;
use std::fs;
use std::path::{Path, PathBuf};

/// Returns the base qol-tray data directory.
///
/// Resolves to `$XDG_DATA_LOCAL_DIR/qol-tray` (or `$XDG_DATA_DIR/qol-tray` as fallback).
pub fn base_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .map(|path| path.join("qol-tray"))
}

/// Returns ordered config root directories for the current qol-tray installation.
///
/// Priority: install-specific dir (from env) > install-specific dir (from active file) > base dir.
pub fn config_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let Some(base) = base_data_dir() else {
        return roots;
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

/// Returns all candidate config file paths for a plugin.
///
/// Searches each config root for `plugins/{name}/config.json` using all provided
/// name variants (e.g. `["plugin-alt-tab", "alt-tab"]`).
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

/// Loads and deserializes plugin config from the first valid config file found.
///
/// Falls back to `T::default()` if no config file exists or all fail to parse.
pub fn load_plugin_config<T: DeserializeOwned + Default>(names: &[&str]) -> T {
    for path in plugin_config_paths(names) {
        let Ok(contents) = fs::read_to_string(&path) else {
            continue;
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
    if valid_install_id(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

pub fn install_id_from_active_file(base_data_dir: &Path) -> Option<String> {
    let content = fs::read_to_string(base_data_dir.join("active-install-id")).ok()?;
    let trimmed = content.trim();
    if valid_install_id(trimmed) {
        Some(trimmed.to_string())
    } else {
        None
    }
}

pub fn valid_install_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
