use std::fs;
use std::path::{Path, PathBuf};

use super::super::AppEntry;

const EXEC_FIELD_CODES: &[&str] = &[
    "%u", "%U", "%f", "%F", "%i", "%c", "%k", "%d", "%D", "%n", "%N", "%v", "%m",
];

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

pub fn app_watch_roots() -> Vec<PathBuf> {
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

    dirs.sort();
    dirs.dedup();
    dirs
}

pub fn load_app_entries() -> Vec<AppEntry> {
    scan_desktop_entries(&app_watch_roots())
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

fn scan_desktop_entries(dirs: &[PathBuf]) -> Vec<AppEntry> {
    let mut entries: Vec<AppEntry> = dirs
        .iter()
        .filter_map(|dir| fs::read_dir(dir).ok())
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "desktop"))
        .filter_map(|p| parse_desktop_entry(&p))
        .collect();

    entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    entries.dedup_by(|a, b| a.name.to_lowercase() == b.name.to_lowercase());
    entries
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
