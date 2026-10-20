use std::path::{Path, PathBuf};

use crate::places::PlaceKind;

const HOME_VARIABLE: &str = "$HOME";

pub(super) fn parse(contents: &str, home: &Path) -> Vec<(PlaceKind, PathBuf)> {
    contents
        .lines()
        .filter_map(|line| parse_line(line, home))
        .collect()
}

fn parse_line(line: &str, home: &Path) -> Option<(PlaceKind, PathBuf)> {
    let line = line.trim();
    if line.starts_with('#') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    let kind = kind_for(key.trim())?;
    let path = resolve(&unquote(value.trim()), home)?;
    Some((kind, path))
}

fn kind_for(key: &str) -> Option<PlaceKind> {
    match key {
        "XDG_DESKTOP_DIR" => Some(PlaceKind::Desktop),
        "XDG_DOCUMENTS_DIR" => Some(PlaceKind::Documents),
        "XDG_DOWNLOAD_DIR" => Some(PlaceKind::Downloads),
        "XDG_MUSIC_DIR" => Some(PlaceKind::Music),
        "XDG_PICTURES_DIR" => Some(PlaceKind::Pictures),
        "XDG_VIDEOS_DIR" => Some(PlaceKind::Videos),
        _ => None,
    }
}

fn unquote(value: &str) -> String {
    let inner = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(value);
    let mut unescaped = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(next) = chars.next() {
        if next != '\\' {
            unescaped.push(next);
            continue;
        }
        if let Some(escaped) = chars.next() {
            unescaped.push(escaped);
        }
    }
    unescaped
}

fn resolve(value: &str, home: &Path) -> Option<PathBuf> {
    if let Some(rest) = value.strip_prefix(HOME_VARIABLE) {
        return in_home(rest, home);
    }
    value.starts_with('/').then(|| PathBuf::from(value))
}

fn in_home(rest: &str, home: &Path) -> Option<PathBuf> {
    if rest.is_empty() {
        return Some(home.to_path_buf());
    }
    let relative = rest.strip_prefix('/')?.trim_start_matches('/');
    if relative.is_empty() {
        return Some(home.to_path_buf());
    }
    Some(home.join(relative))
}
