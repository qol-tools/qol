use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

pub(super) type FingerprintInput = (PathBuf, PathBuf);

pub(super) fn fingerprint_inputs(path: &Path) -> Result<Vec<FingerprintInput>, String> {
    let mut inputs = Vec::new();
    for entry in walker(path) {
        let entry = entry.map_err(|error| format!("Walk error: {}", error))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let relative_path = relative_path(path, entry.path())?;
        if !is_fingerprint_input(&relative_path) {
            continue;
        }
        inputs.push((relative_path, entry.path().to_path_buf()));
    }
    Ok(inputs)
}

fn walker(path: &Path) -> impl Iterator<Item = Result<walkdir::DirEntry, walkdir::Error>> {
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_entry(keep_entry)
}

fn keep_entry(entry: &walkdir::DirEntry) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    !(entry.file_type().is_dir() && should_skip_dir(entry.file_name()))
}

fn relative_path(root: &Path, path: &Path) -> Result<PathBuf, String> {
    path.strip_prefix(root)
        .map(|relative| relative.to_path_buf())
        .map_err(|error| format!("Failed to relativize path: {}", error))
}

fn should_skip_dir(name: &OsStr) -> bool {
    matches!(name.to_str(), Some("target" | ".git" | ".hg" | ".svn"))
}

fn is_fingerprint_input(relative_path: &Path) -> bool {
    let Some(file_name) = relative_path.file_name().and_then(OsStr::to_str) else {
        return false;
    };

    if matches!(
        file_name,
        "Cargo.toml" | "Cargo.lock" | "build.rs" | "rust-toolchain" | "rust-toolchain.toml"
    ) {
        return true;
    }

    if relative_path
        .components()
        .any(|component| component.as_os_str() == OsStr::new(".cargo"))
    {
        return true;
    }

    relative_path
        .extension()
        .and_then(OsStr::to_str)
        .map(|ext| ext.eq_ignore_ascii_case("rs"))
        .unwrap_or(false)
}
