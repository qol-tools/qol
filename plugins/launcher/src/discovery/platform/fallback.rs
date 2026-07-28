use std::path::PathBuf;

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
