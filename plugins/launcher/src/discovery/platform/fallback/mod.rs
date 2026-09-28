use std::path::{Path, PathBuf};

use super::super::details::AppDetails;
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

pub fn app_details(_entry: &AppEntry) -> AppDetails {
    AppDetails::default()
}

pub fn file_icon(_path: &Path) -> Option<PathBuf> {
    None
}

pub fn recent_files(_limit: usize) -> Vec<PathBuf> {
    Vec::new()
}
