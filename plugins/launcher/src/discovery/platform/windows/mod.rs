use std::path::{Path, PathBuf};

use qol_watch::WatchRoot;

use super::super::details::{AppAbout, AppFace};
use super::super::AppEntry;
use super::AppRoot;

pub fn cache_dir() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(std::env::temp_dir()))
}

pub fn app_roots() -> Vec<AppRoot> {
    qol_apps::start_menu::start_menu_roots()
}

pub fn scan_root(root: &AppRoot) -> Vec<AppEntry> {
    qol_apps::start_menu::scan_start_menu_root(root)
}

pub fn app_watch_root(root: &AppRoot) -> WatchRoot {
    WatchRoot::deep(root.path.clone())
}

pub fn app_change(_root: &AppRoot, path: &Path) -> Option<PathBuf> {
    match path.extension() {
        Some(_) if !qol_apps::start_menu::is_start_menu_shortcut(path) => None,
        _ => Some(path.to_path_buf()),
    }
}

pub fn file_watch_roots() -> Vec<PathBuf> {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    vec![
        PathBuf::from(format!("{home}\\Desktop")),
        PathBuf::from(format!("{home}\\Documents")),
        PathBuf::from(format!("{home}\\Downloads")),
    ]
}

pub fn app_face(_entry: &AppEntry) -> AppFace {
    AppFace::default()
}

pub fn app_about(_entry: &AppEntry) -> AppAbout {
    AppAbout::default()
}

pub fn file_icon(_path: &Path) -> Option<PathBuf> {
    None
}

pub fn recent_files(_limit: usize) -> Vec<PathBuf> {
    Vec::new()
}
