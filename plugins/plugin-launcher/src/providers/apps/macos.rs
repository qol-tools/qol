use std::fs;
use std::path::{Path, PathBuf};

use super::{AppEntry, AppsProvider};

pub struct MacosAppsProvider;

impl AppsProvider for MacosAppsProvider {
    fn load_entries(&self) -> Vec<AppEntry> {
        let mut entries = Vec::new();
        for dir in app_dirs() {
            collect_apps(&dir, 0, &mut entries);
        }
        entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        entries.dedup_by(|a, b| a.name.to_lowercase() == b.name.to_lowercase());
        entries
    }
}

/// .app bundles sit at the root of /Applications or one level deep
/// (e.g. /Applications/Utilities/). Deeper traversal enters bundle
/// internals and unrelated directory trees — avoid it.
const MAX_DEPTH: usize = 1;

fn app_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/Applications")];
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(format!("{home}/Applications")));
    }
    dirs.push(PathBuf::from("/System/Applications"));
    dirs
}

fn collect_apps(dir: &Path, depth: usize, out: &mut Vec<AppEntry>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        if path.extension().is_some_and(|ext| ext == "app") {
            if let Some(app_entry) = parse_app_bundle(&path) {
                out.push(app_entry);
            }
            // Never descend into .app bundles
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') {
            continue;
        }
        collect_apps(&path, depth + 1, out);
    }
}

fn parse_app_bundle(path: &Path) -> Option<AppEntry> {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())?
        .to_string();

    // exec is unused on macOS (launch goes through open_path_detached),
    // but the field is required by the shared AppEntry struct.
    let exec = format!("open {}", path.display());

    Some(AppEntry {
        name,
        exec,
        path: path.to_path_buf(),
    })
}
