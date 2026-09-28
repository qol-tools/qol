mod cask;
mod icon;
mod spotlight;

use std::path::{Path, PathBuf};

use super::super::details::{AppAbout, AppFace, Package};
use super::super::AppEntry;
use super::AppRoot;

pub fn cache_dir() -> Option<PathBuf> {
    qol_apps::macos_cache_dir()
}

pub fn app_roots() -> Vec<AppRoot> {
    qol_apps::macos_launcher_roots()
}

pub fn scan_root(root: &AppRoot) -> Vec<AppEntry> {
    qol_apps::scan_macos_launcher_root(root)
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

pub fn app_face(entry: &AppEntry) -> AppFace {
    let category = qol_apps::read_macos_bundle_facts(&entry.path).category;
    AppFace {
        icon: icon::icon_path(&entry.path),
        description: cask::owner_of(&entry.path)
            .and_then(|cask| cask.summary)
            .or_else(|| category.as_deref().and_then(kind)),
    }
}

pub fn app_about(entry: &AppEntry) -> AppAbout {
    let bundle = qol_apps::read_macos_bundle_facts(&entry.path);
    let cask = cask::owner_of(&entry.path);
    let indexed = spotlight::facts(&entry.path);
    let kind = bundle.category.as_deref().and_then(kind);
    let package = cask
        .as_ref()
        .map(|cask| cask.token.clone())
        .or(bundle.identifier)
        .map(|name| Package {
            name,
            version: bundle.version,
            size: indexed.size,
            installed: cask
                .as_ref()
                .and_then(|cask| cask.installed)
                .or(indexed.added),
        });
    AppAbout {
        binary: bundle
            .executable
            .map(|name| entry.path.join("Contents/MacOS").join(name))
            .filter(|binary| binary.is_file()),
        kind,
        source: if cask.is_some() {
            Some("Homebrew")
        } else {
            source_of(&entry.path)
        },
        package,
        developer: bundle.copyright.as_deref().and_then(owner),
        website: cask.and_then(|cask| cask.website),
        ..AppAbout::default()
    }
}

fn source_of(path: &Path) -> Option<&'static str> {
    if path.join("Contents/_MASReceipt/receipt").is_file() {
        return Some("App Store");
    }
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    real.starts_with("/System").then_some("macOS")
}

fn kind(category: &str) -> Option<String> {
    let words = category
        .strip_prefix("public.app-category.")?
        .split('-')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = words.chars();
    let first = chars.next()?;
    Some(first.to_uppercase().chain(chars).collect())
}

fn owner(copyright: &str) -> Option<String> {
    let line = copyright.lines().next().unwrap_or(copyright);
    let line = line.split("\\n").next().unwrap_or(line);
    let line = match line.to_lowercase().find("all rights reserved") {
        Some(at) => &line[..at],
        None => line,
    };
    let words: Vec<&str> = line
        .split_whitespace()
        .map(|word| word.trim_start_matches(['©', 'Ⓒ']))
        .filter(|word| {
            !word.is_empty()
                && !word.eq_ignore_ascii_case("copyright")
                && !word.eq_ignore_ascii_case("(c)")
                && !is_years(word)
        })
        .collect();
    let name = words.join(" ");
    let name = name.trim_matches([',', '.', ' ']);
    (!name.is_empty()).then(|| name.to_owned())
}

fn is_years(word: &str) -> bool {
    let word = word.trim_end_matches([',', '.']);
    word.chars().any(|ch| ch.is_ascii_digit())
        && word
            .chars()
            .all(|ch| ch.is_ascii_digit() || ch == '-' || ch == '–')
}

pub fn file_icon(_path: &Path) -> Option<PathBuf> {
    None
}

pub fn recent_files(_limit: usize) -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_name_the_app_store_category_in_plain_words() {
        assert_eq!(
            kind("public.app-category.developer-tools").as_deref(),
            Some("Developer tools")
        );
        assert_eq!(
            kind("public.app-category.productivity").as_deref(),
            Some("Productivity")
        );
        assert_eq!(kind("com.acme.custom").as_deref(), None);
        assert_eq!(kind("public.app-category.").as_deref(), None);
    }

    #[test]
    fn owners_drop_the_copyright_words_years_and_reservation() {
        for (copyright, expected) in [
            (
                "Copyright (C) 2026 Microsoft. All rights reserved",
                Some("Microsoft"),
            ),
            ("Copyright 2025, Kovid Goyal", Some("Kovid Goyal")),
            (
                "© 2019–2026 Apple Inc. All rights reserved.",
                Some("Apple Inc"),
            ),
            (
                "Copyright © 2015-2025 Docker Inc.All Rights Reserved.",
                Some("Docker Inc"),
            ),
            (
                "Copyright © 2002-2025 Apple Inc.\\n All Rights Reserved.",
                Some("Apple Inc"),
            ),
            (
                "Copyright © 2007-2025 Apple Inc.\nAll rights reserved.",
                Some("Apple Inc"),
            ),
            (
                "Copyright © Microsoft Corporation 2026. All rights reserved.",
                Some("Microsoft Corporation"),
            ),
            (
                "Copyright ©2012-2026 Zoom Communications, Inc. All rights reserved",
                Some("Zoom Communications, Inc"),
            ),
            ("Copyright Ⓒ waydabber/KodeON", Some("waydabber/KodeON")),
            ("2025 Anthropic PBC", Some("Anthropic PBC")),
            ("All rights reserved.", None),
        ] {
            assert_eq!(owner(copyright).as_deref(), expected, "{copyright}");
        }
    }
}
