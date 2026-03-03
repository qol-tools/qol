use std::path::PathBuf;

use crate::desktop_entry;

use super::{AppEntry, AppsProvider};

pub struct LinuxAppsProvider;

impl AppsProvider for LinuxAppsProvider {
    fn load_entries(&self) -> Vec<AppEntry> {
        desktop_entry::scan(&application_dirs())
    }
}

pub(super) fn application_dirs() -> Vec<PathBuf> {
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
