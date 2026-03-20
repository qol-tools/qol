use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::super::AppEntry;

pub fn cache_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|home| PathBuf::from(format!("{home}/Library/Caches")))
}

pub fn app_watch_roots() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/Applications")];
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(format!("{home}/Applications")));
    }
    dirs.push(PathBuf::from("/System/Applications"));
    dirs.push(PathBuf::from("/System/Library/CoreServices"));
    dirs.push(PathBuf::from("/System/Library/CoreServices/Applications"));
    dirs.push(PathBuf::from("/System/Library/PreferencePanes"));
    dirs
}

pub fn load_app_entries() -> Vec<AppEntry> {
    let dirs = app_watch_roots();
    let mut entries = spotlight_entries(&dirs);
    for dir in &dirs {
        collect_apps(dir, 0, &mut entries);
    }
    entries.retain(|e| !is_excluded_launcher(&e.name));
    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    entries.dedup_by(|a, b| a.name.to_lowercase() == b.name.to_lowercase());
    entries
}

pub fn file_watch_roots() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    vec![
        PathBuf::from(format!("{home}/Desktop")),
        PathBuf::from(format!("{home}/Documents")),
        PathBuf::from(format!("{home}/Downloads")),
        PathBuf::from(format!("{home}/Projects")),
        PathBuf::from(format!("{home}/.config")),
    ]
}

const APP_MAX_DEPTH: usize = 1;
const EXCLUDED_LAUNCHERS: &[&str] = &["Spotlight", "Launchpad"];

fn is_excluded_launcher(name: &str) -> bool {
    EXCLUDED_LAUNCHERS
        .iter()
        .any(|e| e.eq_ignore_ascii_case(name))
}

fn spotlight_entries(dirs: &[PathBuf]) -> Vec<AppEntry> {
    let mut command = Command::new("mdfind");
    for dir in dirs {
        command.arg("-onlyin").arg(dir);
    }
    command.arg(
        "kMDItemContentType == 'com.apple.application-bundle' || kMDItemContentType == 'com.apple.systempreference.prefpane'",
    );

    let Ok(output) = command.output() else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .filter_map(|path| parse_spotlight_bundle(&path))
        .collect()
}

fn parse_spotlight_bundle(path: &Path) -> Option<AppEntry> {
    if !path.is_dir() {
        return None;
    }
    if path
        .components()
        .any(|component| component.as_os_str() == OsStr::new("Contents"))
    {
        return None;
    }
    if !path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("app") || ext.eq_ignore_ascii_case("prefPane"))
        .unwrap_or(false)
    {
        return None;
    }
    parse_app_bundle(path)
}

fn collect_apps(dir: &Path, depth: usize, out: &mut Vec<AppEntry>) {
    if depth > APP_MAX_DEPTH {
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
        if path
            .extension()
            .is_some_and(|ext| ext == "app" || ext == "prefPane")
        {
            if let Some(app_entry) = parse_app_bundle(&path) {
                out.push(app_entry);
            }
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
    let name = path.file_stem().and_then(|s| s.to_str())?.to_string();

    Some(AppEntry {
        name,
        exec: vec!["open".into(), path.display().to_string()],
        path: path.to_path_buf(),
    })
}
