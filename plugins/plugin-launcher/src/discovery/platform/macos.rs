use std::fs;
use std::path::{Path, PathBuf};

use super::super::AppEntry;
use super::AppRoot;

const APP_ROOT_DEPTH: usize = 2;
const EXCLUDED_LAUNCHERS: &[&str] = &["Spotlight", "Launchpad"];

pub fn cache_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|home| PathBuf::from(format!("{home}/Library/Caches")))
}

pub fn app_roots() -> Vec<AppRoot> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut roots = vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Library/CoreServices"),
        PathBuf::from("/System/Library/CoreServices/Applications"),
        PathBuf::from("/System/Library/PreferencePanes"),
    ];
    if !home.is_empty() {
        roots.push(PathBuf::from(format!("{home}/Applications")));
    }
    roots
        .into_iter()
        .map(|path| AppRoot {
            path,
            max_depth: APP_ROOT_DEPTH,
        })
        .filter(|r| r.path.is_dir())
        .collect()
}

pub fn scan_root(root: &AppRoot) -> Vec<AppEntry> {
    let mut out = Vec::new();
    collect_apps(&root.path, 0, root.max_depth, &mut out);
    out.retain(|e| !is_excluded_launcher(&e.name));
    out
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

fn is_excluded_launcher(name: &str) -> bool {
    EXCLUDED_LAUNCHERS
        .iter()
        .any(|e| e.eq_ignore_ascii_case(name))
}

fn collect_apps(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<AppEntry>) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        if let Some(app_entry) = parse_app_path(&path) {
            out.push(app_entry);
            continue;
        }
        if !path.is_dir() || depth >= max_depth {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') {
            continue;
        }
        collect_apps(&path, depth + 1, max_depth, out);
    }
}

fn parse_app_path(path: &Path) -> Option<AppEntry> {
    if !is_supported_app_path(path) {
        return None;
    }
    let name = path.file_stem().and_then(|s| s.to_str())?.to_string();

    Some(AppEntry {
        name,
        exec: vec!["open".into(), path.display().to_string()],
        path: path.to_path_buf(),
    })
}

fn is_supported_app_path(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("app") || ext.eq_ignore_ascii_case("prefPane"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn collect_apps_includes_regular_app_files() {
        let root = temp_path("regular-app-file");
        let apps = root.join("Applications");
        let qol = apps.join("QoL");
        fs::create_dir_all(&qol).unwrap();
        File::create(qol.join("ChatGPT.app")).unwrap();

        let mut entries = Vec::new();
        collect_apps(&apps, 0, APP_ROOT_DEPTH, &mut entries);

        assert!(entries.iter().any(|entry| entry.name == "ChatGPT"));
        fs::remove_dir_all(&root).unwrap();
    }

    fn temp_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("launcher-macos-{name}-{nanos}"))
    }
}
