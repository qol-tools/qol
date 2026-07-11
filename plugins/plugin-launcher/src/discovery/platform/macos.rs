use std::path::PathBuf;

use super::super::AppEntry;
use super::AppRoot;

pub fn cache_dir() -> Option<PathBuf> {
    qol_apps::macos_cache_dir()
}

pub fn app_roots() -> Vec<AppRoot> {
    qol_apps::macos_launcher_roots()
}

pub fn scan_root(root: &AppRoot) -> Vec<AppEntry> {
    qol_apps::scan_macos_launcher_root(root)
}

pub fn file_watch_roots() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    vec![
        PathBuf::from(format!("{home}/Desktop")),
        PathBuf::from(format!("{home}/Documents")),
        PathBuf::from(format!("{home}/Downloads")),
        PathBuf::from(format!("{home}/Projects")),
        PathBuf::from(format!("{home}/.config")),
    ]
}
