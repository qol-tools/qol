use std::fs;
use std::path::{Path, PathBuf};

use super::FileEntry;

const MAX_FILES: usize = 8_000;
const MAX_DEPTH: usize = 6;

pub(crate) fn scan_files(roots: Vec<PathBuf>) -> Vec<FileEntry> {
    let mut files = Vec::new();
    for root in roots {
        if files.len() >= MAX_FILES {
            break;
        }
        collect_files(&root, 0, &mut files);
    }
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    files
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
            let is_hidden = path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.starts_with('.'));
            if !is_hidden {
                collect_files(&path, depth + 1, out);
            }
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
