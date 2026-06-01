use std::fs;
use std::path::{Path, PathBuf};

use super::super::AppEntry;
use super::AppRoot;

const EXEC_FIELD_CODES: &[&str] = &[
    "%u", "%U", "%f", "%F", "%i", "%c", "%k", "%d", "%D", "%n", "%N", "%v", "%m",
];

const XDG_ROOT_DEPTH: usize = 1;
const LOOSE_ROOT_DEPTH: usize = 2;

pub fn cache_dir() -> Option<PathBuf> {
    std::env::var("XDG_CACHE_HOME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .map(|home| PathBuf::from(format!("{home}/.cache")))
        })
}

pub fn app_roots() -> Vec<AppRoot> {
    let mut roots: Vec<AppRoot> = xdg_app_dirs()
        .into_iter()
        .map(|path| AppRoot {
            path,
            max_depth: XDG_ROOT_DEPTH,
        })
        .collect();

    roots.extend(loose_install_dirs().into_iter().map(|path| AppRoot {
        path,
        max_depth: LOOSE_ROOT_DEPTH,
    }));

    roots.sort_by(|a, b| a.path.cmp(&b.path));
    roots.dedup_by(|a, b| a.path == b.path);
    roots.retain(|r| r.path.is_dir());
    roots
}

pub fn scan_root(root: &AppRoot) -> Vec<AppEntry> {
    let mut out = Vec::new();
    walk_for_desktop(&root.path, 0, root.max_depth, &mut out);
    out
}

pub fn file_watch_roots() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();

    let mut roots = vec![
        PathBuf::from(format!("{home}/Desktop")),
        PathBuf::from(format!("{home}/Documents")),
        PathBuf::from(format!("{home}/Downloads")),
        PathBuf::from(format!("{home}/Projects")),
    ];

    if let Some(config_root) = xdg_config_root(&home) {
        roots.push(config_root);
    }

    roots.extend(user_dirs_from_config(&home));
    roots.sort();
    roots.dedup();
    roots
}

fn xdg_app_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let data_home = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| format!("{home}/.local/share"));

    let mut dirs = vec![
        PathBuf::from("/usr/share/applications"),
        PathBuf::from("/usr/local/share/applications"),
        PathBuf::from(format!("{data_home}/applications")),
        PathBuf::from("/var/lib/flatpak/exports/share/applications"),
        PathBuf::from("/var/lib/snapd/desktop/applications"),
    ];

    if let Ok(extra) = std::env::var("XDG_DATA_DIRS") {
        for segment in extra.split(':') {
            let trimmed = segment.trim();
            if trimmed.is_empty() {
                continue;
            }
            dirs.push(PathBuf::from(format!("{trimmed}/applications")));
        }
    }

    dirs
}

fn loose_install_dirs() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut dirs = vec![PathBuf::from("/opt")];
    if !home.is_empty() {
        dirs.push(PathBuf::from(format!("{home}/.local")));
        dirs.push(PathBuf::from(format!("{home}/Applications")));
    }
    dirs
}

fn walk_for_desktop(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<AppEntry>) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.flatten() {
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        if file_type.is_file() && path.extension().is_some_and(|ext| ext == "desktop") {
            if let Some(parsed) = parse_desktop_entry(&path) {
                out.push(parsed);
            }
            continue;
        }

        if !file_type.is_dir() || depth >= max_depth {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with('.') || (depth == 0 && name == "share") {
            continue;
        }
        walk_for_desktop(&path, depth + 1, max_depth, out);
    }
}

fn parse_desktop_entry(path: &Path) -> Option<AppEntry> {
    let content = fs::read_to_string(path).ok()?;

    let field = |prefix: &str| {
        content
            .lines()
            .find(|l| l.starts_with(prefix))
            .map(|l| l[prefix.len()..].to_string())
    };

    if content
        .lines()
        .any(|l| l == "NoDisplay=true" || l == "Hidden=true")
    {
        return None;
    }

    let exec_raw = field("Exec=")?;
    let exec = shell_words::split(&exec_raw)
        .ok()?
        .into_iter()
        .filter(|token| !EXEC_FIELD_CODES.contains(&token.as_str()))
        .collect();

    Some(AppEntry {
        name: field("Name=")?,
        exec,
        path: path.to_path_buf(),
    })
}

fn xdg_config_root(home: &str) -> Option<PathBuf> {
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
    let Ok(content) = fs::read_to_string(config_path) else {
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
