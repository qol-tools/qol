use std::path::{Path, PathBuf};

use super::super::details::{AppAbout, AppFace};
use super::super::AppEntry;
use super::AppRoot;

pub fn cache_dir() -> Option<PathBuf> {
    None
}

pub fn app_roots() -> Vec<AppRoot> {
    Vec::new()
}

pub fn scan_root(_root: &AppRoot) -> Vec<AppEntry> {
    Vec::new()
}

pub fn file_watch_roots() -> Vec<PathBuf> {
    Vec::new()
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
