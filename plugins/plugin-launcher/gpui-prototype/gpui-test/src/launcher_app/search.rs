use crate::desktop_entry::DesktopEntry;
use crate::{fuzzy_match, FuzzyMatch};
use std::fs;
use std::path::{Path, PathBuf};

use super::state::SearchMode;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
}

pub enum ResultItem<'a> {
    App(&'a DesktopEntry),
    File(&'a FileEntry),
}

impl<'a> ResultItem<'a> {
    pub fn name(&self) -> &str {
        match self {
            Self::App(entry) => &entry.name,
            Self::File(entry) => &entry.name,
        }
    }
}

pub struct Scored<'a> {
    pub item: ResultItem<'a>,
    pub m: FuzzyMatch,
}

pub fn filtered<'a>(
    app_entries: &'a [DesktopEntry],
    file_entries: &'a [FileEntry],
    query: &str,
    mode: SearchMode,
) -> Vec<Scored<'a>> {
    if query.trim().is_empty() {
        return Vec::new();
    }

    let mut results: Vec<Scored<'_>> = match mode {
        SearchMode::Apps => app_entries
            .iter()
            .filter_map(|entry| {
                fuzzy_match(query, &entry.name).map(|m| Scored {
                    item: ResultItem::App(entry),
                    m,
                })
            })
            .collect(),
        SearchMode::Files => file_entries
            .iter()
            .filter_map(|entry| {
                fuzzy_match(query, &entry.name).map(|m| Scored {
                    item: ResultItem::File(entry),
                    m,
                })
            })
            .collect(),
    };
    results.sort_by_key(|s| s.m.score);
    results
}

const MAX_FILES: usize = 8_000;
const MAX_DEPTH: usize = 6;

pub fn scan_files() -> Vec<FileEntry> {
    let mut files = Vec::new();
    for root in file_roots() {
        if files.len() >= MAX_FILES {
            break;
        }
        collect_files(&root, 0, &mut files);
    }
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    files
}

fn file_roots() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    vec![
        PathBuf::from(format!("{home}/Desktop")),
        PathBuf::from(format!("{home}/Documents")),
        PathBuf::from(format!("{home}/Downloads")),
        PathBuf::from(format!("{home}/Projects")),
        PathBuf::from(format!("{home}/.config")),
    ]
}

fn collect_files(dir: &Path, depth: usize, out: &mut Vec<FileEntry>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };

    for entry in read_dir.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') {
                    continue;
                }
            }
            collect_files(&path, depth + 1, out);
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        out.push(FileEntry {
            name: name.to_string(),
            path,
        });
    }
}
