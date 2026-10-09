use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::{AppEntry, AppRoot};

const SHORTCUT_EXTENSION: &str = "lnk";

const NOISE_PREFIXES: &[&str] = &["uninstall", "un-install"];
const NOISE_SUFFIXES: &[&str] = &["uninstall", "help", "documentation", "release notes"];
const NOISE_WORDS: &[&str] = &["readme", "read me"];

mod platform;

pub fn start_menu_roots() -> Vec<AppRoot> {
    platform::program_roots()
}

pub fn scan_start_menu_root(root: &AppRoot) -> Vec<AppEntry> {
    let mut shortcuts = platform::shortcut_paths(root);
    shortcuts.sort();
    entries_from_shortcuts(&shortcuts)
}

pub fn is_start_menu_shortcut(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(SHORTCUT_EXTENSION))
}

/// Builds app entries from shortcut paths, in order. When two shortcuts share a
/// name (one app listed under several folders, or in both the user and common
/// Start Menu), the first one wins.
pub fn entries_from_shortcuts(shortcuts: &[PathBuf]) -> Vec<AppEntry> {
    let mut seen = HashSet::new();
    shortcuts
        .iter()
        .filter_map(|path| shortcut_entry(path))
        .filter(|entry| seen.insert(entry.name.to_lowercase()))
        .collect()
}

fn shortcut_entry(path: &Path) -> Option<AppEntry> {
    if !is_start_menu_shortcut(path) {
        return None;
    }
    let name = path.file_stem()?.to_str()?.trim();
    if name.is_empty() || is_noise(name) {
        return None;
    }
    Some(AppEntry {
        name: name.to_string(),
        exec: vec![path.to_string_lossy().into_owned()],
        path: path.to_path_buf(),
    })
}

fn is_noise(name: &str) -> bool {
    let lower = name.to_lowercase();
    let prefixed = NOISE_PREFIXES.iter().any(|noise| lower.starts_with(noise));
    let suffixed = NOISE_SUFFIXES.iter().any(|noise| {
        lower
            .strip_suffix(noise)
            .is_some_and(|rest| rest.ends_with(' '))
    });
    let worded = NOISE_WORDS.iter().any(|noise| lower.contains(noise));
    prefixed || suffixed || worded
}

#[cfg(test)]
mod tests;
