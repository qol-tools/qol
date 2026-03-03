use std::path::PathBuf;

use super::super::AppEntry;

pub fn cache_dir() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(std::env::temp_dir()))
}

pub fn app_watch_roots() -> Vec<PathBuf> {
    Vec::new()
}

pub fn load_app_entries() -> Vec<AppEntry> {
    Vec::new()
}

pub fn file_watch_roots() -> Vec<PathBuf> {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    vec![
        PathBuf::from(format!("{home}\\Desktop")),
        PathBuf::from(format!("{home}\\Documents")),
        PathBuf::from(format!("{home}\\Downloads")),
    ]
}
