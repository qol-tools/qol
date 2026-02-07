use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::providers::files::FileEntry;

const CACHE_VERSION: &str = "v1";
const CACHE_TTL: Duration = Duration::from_secs(60 * 15);
const CACHE_FILE_NAME: &str = "launcher-files-index-v1.tsv";

pub fn load(roots: &[PathBuf]) -> Option<Vec<FileEntry>> {
    let path = cache_path()?;
    if is_stale(&path) {
        return None;
    }
    let file = fs::File::open(path).ok()?;
    let mut lines = BufReader::new(file).lines();
    let header = lines.next()?.ok()?;
    let expected = format!("{CACHE_VERSION}\t{}", roots_fingerprint(roots));
    if header != expected {
        return None;
    }

    let mut entries = Vec::new();
    for line in lines {
        let Ok(line) = line else {
            return None;
        };
        let Some((name, path)) = line.split_once('\t') else {
            continue;
        };
        if name.is_empty() || path.is_empty() {
            continue;
        }
        entries.push(FileEntry {
            name: name.to_string(),
            path: PathBuf::from(path),
        });
    }
    if entries.is_empty() {
        None
    } else {
        Some(entries)
    }
}

pub fn store(roots: &[PathBuf], entries: &[FileEntry]) {
    let Some(path) = cache_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let Ok(mut file) = fs::File::create(path) else {
        return;
    };

    if writeln!(file, "{CACHE_VERSION}\t{}", roots_fingerprint(roots)).is_err() {
        return;
    }
    for entry in entries {
        let name = entry.name.replace('\t', " ");
        let path = entry.path.to_string_lossy().replace('\t', " ");
        if writeln!(file, "{name}\t{path}").is_err() {
            return;
        }
    }
}

fn cache_path() -> Option<PathBuf> {
    let base = std::env::var("XDG_CACHE_HOME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .map(|home| PathBuf::from(format!("{home}/.cache")))
        })?;
    Some(base.join("gpui-test").join(CACHE_FILE_NAME))
}

fn is_stale(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return true;
    };
    let Ok(modified) = meta.modified() else {
        return true;
    };
    let Ok(age) = SystemTime::now().duration_since(modified) else {
        return true;
    };
    age > CACHE_TTL
}

fn roots_fingerprint(roots: &[PathBuf]) -> u64 {
    let mut normalized: Vec<String> = roots
        .iter()
        .map(|p| p.to_string_lossy().to_lowercase())
        .collect();
    normalized.sort();
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    hasher.finish()
}
