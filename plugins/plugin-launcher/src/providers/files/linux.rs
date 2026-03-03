use std::path::PathBuf;

use crate::platform::{self, LinuxDisplayBackend};

use super::linux_index;
use super::fallback::scan_files;
use super::{FileEntry, FilesProvider};

pub struct LinuxFilesProvider;

impl FilesProvider for LinuxFilesProvider {
    fn load_entries(&self) -> Vec<FileEntry> {
        let roots = file_roots();
        if let Some(entries) = linux_index::load(&roots) {
            return entries;
        }
        let entries = scan_files(roots.clone());
        linux_index::store(&roots, &entries);
        entries
    }
}

pub(super) fn file_roots() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();

    let mut roots = vec![
        PathBuf::from(format!("{home}/Desktop")),
        PathBuf::from(format!("{home}/Documents")),
        PathBuf::from(format!("{home}/Downloads")),
        PathBuf::from(format!("{home}/Projects")),
    ];

    if let Some(config_root) = config_dir(&home) {
        roots.push(config_root);
    }

    roots.extend(user_dirs_from_config(&home));

    match platform::linux_display_backend() {
        LinuxDisplayBackend::X11 | LinuxDisplayBackend::Wayland | LinuxDisplayBackend::Unknown => {
            roots.sort();
            roots.dedup();
            roots
        }
    }
}

fn config_dir(home: &str) -> Option<PathBuf> {
    std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            if home.is_empty() {
                None
            } else {
                Some(PathBuf::from(format!("{home}/.config")))
            }
        })
}

fn user_dirs_from_config(home: &str) -> Vec<PathBuf> {
    if home.is_empty() {
        return Vec::new();
    }

    let config_path = PathBuf::from(format!("{home}/.config/user-dirs.dirs"));
    let Ok(content) = std::fs::read_to_string(config_path) else {
        return Vec::new();
    };

    content
        .lines()
        .filter_map(|line| parse_user_dir_line(line, home))
        .collect()
}

fn parse_user_dir_line(line: &str, home: &str) -> Option<PathBuf> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let (_key, raw_value) = trimmed.split_once('=')?;
    let mut value = raw_value.trim().trim_matches('"').to_string();
    if value.is_empty() {
        return None;
    }
    value = value.replace("$HOME", home);
    Some(PathBuf::from(value))
}
