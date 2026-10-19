use std::path::{is_separator, Component, Path, PathBuf, MAIN_SEPARATOR};

use super::prefix::starts_with_ignoring_case;
use crate::listing::{needs_mark, Listing};

const HOME: &str = "~";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathInput {
    pub parent: Option<PathBuf>,
    pub segment: String,
}

impl PathInput {
    pub fn parse(input: &str, home: &Path) -> Self {
        let Some(index) = input.rfind(is_separator) else {
            return Self {
                parent: None,
                segment: input.to_string(),
            };
        };
        Self {
            parent: Some(expand_home(&input[..=index], home)),
            segment: input[index + 1..].to_string(),
        }
    }
}

pub fn expand_home(input: &str, home: &Path) -> PathBuf {
    let Some(rest) = input.strip_prefix(HOME) else {
        return PathBuf::from(input);
    };
    if rest.is_empty() {
        return home.to_path_buf();
    }
    if !rest.starts_with(is_separator) {
        return PathBuf::from(input);
    }
    let rest = rest.trim_start_matches(is_separator);
    if rest.is_empty() {
        return home.to_path_buf();
    }
    home.join(rest)
}

pub fn resolve(input: &str, home: &Path, cwd: &Path) -> PathBuf {
    let path = expand_home(input, home);
    if path.is_absolute() {
        return collapse(&path);
    }
    collapse(&cwd.join(path))
}

pub fn collapse(path: &Path) -> PathBuf {
    let mut collapsed = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                collapsed.pop();
            }
            Component::CurDir => {}
            other => collapsed.push(other),
        }
    }
    collapsed
}

pub fn display_with_home(path: &Path, home: &Path) -> String {
    let Ok(rest) = path.strip_prefix(home) else {
        return path.display().to_string();
    };
    if rest.as_os_str().is_empty() {
        return HOME.to_string();
    }
    format!("{HOME}{MAIN_SEPARATOR}{}", rest.display())
}

pub fn child_toward(ancestor: &Path, folder: &Path) -> Option<PathBuf> {
    let first = folder.strip_prefix(ancestor).ok()?.components().next()?;
    Some(ancestor.join(first))
}

pub fn suggestions<'a>(parent: &'a Listing, segment: &str, show_hidden: bool) -> Vec<&'a str> {
    parent
        .visible(show_hidden || segment.starts_with('.'))
        .into_iter()
        .filter(|entry| entry.opens_as_folder)
        .filter_map(|entry| entry.name.to_str())
        .filter(|name| !name.chars().any(needs_mark))
        .filter(|name| starts_with_ignoring_case(name, segment))
        .collect()
}
