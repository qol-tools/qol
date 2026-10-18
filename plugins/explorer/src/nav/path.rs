use std::path::{is_separator, Path, PathBuf, MAIN_SEPARATOR};

use super::prefix::starts_with_ignoring_case;
use crate::listing::Listing;

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
        return path;
    }
    cwd.join(path)
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

pub fn suggestions<'a>(parent: &'a Listing, segment: &str, show_hidden: bool) -> Vec<&'a str> {
    parent
        .visible(show_hidden)
        .into_iter()
        .filter(|entry| entry.opens_as_folder)
        .map(|entry| entry.display.as_str())
        .filter(|name| starts_with_ignoring_case(name, segment))
        .collect()
}
