mod appstream;
mod icons;
mod package;
mod recent;

use std::fs;
use std::path::{Path, PathBuf};

use qol_watch::WatchRoot;

use super::super::details::{AppAbout, AppFace};
use super::super::AppEntry;
use super::AppRoot;

pub fn cache_dir() -> Option<PathBuf> {
    qol_apps::desktop::xdg_cache_dir()
}

pub fn app_roots() -> Vec<AppRoot> {
    qol_apps::desktop::linux_app_roots()
}

pub fn scan_root(root: &AppRoot) -> Vec<AppEntry> {
    qol_apps::desktop::scan_desktop_root(root)
}

pub fn app_watch_root(root: &AppRoot) -> WatchRoot {
    if root.watch_recursive() {
        WatchRoot::deep(root.path.clone())
    } else {
        WatchRoot::shallow(root.path.clone())
    }
}

pub fn app_change(_root: &AppRoot, path: &Path) -> Option<PathBuf> {
    match path.extension() {
        Some(extension) if extension != "desktop" => None,
        _ => Some(path.to_path_buf()),
    }
}

pub fn app_face(entry: &AppEntry) -> AppFace {
    let Some(field) = desktop_fields(entry) else {
        return AppFace::default();
    };
    AppFace {
        icon: field("Icon=").and_then(|name| icons::icon_path(&name)),
        description: description(&field),
    }
}

pub fn app_about(entry: &AppEntry) -> AppAbout {
    let Some(field) = desktop_fields(entry) else {
        return AppAbout::default();
    };
    about(
        entry,
        description(&field).as_deref(),
        field("Exec="),
        field("Categories="),
    )
}

fn desktop_fields(entry: &AppEntry) -> Option<impl Fn(&str) -> Option<String>> {
    let content = fs::read_to_string(&entry.path).ok()?;
    Some(move |key: &str| {
        qol_apps::desktop::desktop_field(&content, key)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
}

fn description(field: &impl Fn(&str) -> Option<String>) -> Option<String> {
    field("Comment=").or_else(|| field("GenericName="))
}

fn about(
    entry: &AppEntry,
    description: Option<&str>,
    command: Option<String>,
    categories: Option<String>,
) -> AppAbout {
    let component = entry
        .path
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(appstream::component)
        .unwrap_or_default();
    let record = package::owner_of(&entry.path);
    let source = if record.is_some() {
        Some("apt")
    } else {
        source_of(&entry.path)
    };
    let (package, facts) = match record {
        Some(record) => (Some(record.package.clone()), record),
        None => (None, package::Record::default()),
    };
    let program = program(&entry.exec);
    AppAbout {
        summary: [component.summary, facts.summary]
            .into_iter()
            .flatten()
            .find(|summary| {
                description.is_none_or(|description| !summary.eq_ignore_ascii_case(description))
            }),
        long: component.long.or(facts.long),
        binary: program.and_then(binary),
        command,
        kind: categories.as_deref().and_then(kind),
        source,
        package,
        licence: component.licence,
        developer: component.developer.or(facts.maintainer),
        website: component.website.or(facts.website),
        asks_password: program.is_some_and(|program| ELEVATE.contains(&program)),
    }
}

const ELEVATE: [&str; 5] = ["pkexec", "gksu", "gksudo", "kdesu", "sudo"];
const TOOLKITS: [&str; 7] = ["GTK", "GNOME", "KDE", "Qt", "XFCE", "MATE", "Application"];

fn program(exec: &[String]) -> Option<&str> {
    exec.iter()
        .map(String::as_str)
        .find(|token| *token != "env" && !token.contains('='))
}

fn binary(program: &str) -> Option<PathBuf> {
    let path = Path::new(program);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    let search = std::env::var_os("PATH")?;
    std::env::split_paths(&search)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

fn source_of(path: &Path) -> Option<&'static str> {
    let path = path.to_string_lossy();
    if path.contains("/flatpak/exports/") {
        Some("flatpak")
    } else if path.starts_with("/var/lib/snapd/") {
        Some("snap")
    } else {
        None
    }
}

fn kind(categories: &str) -> Option<String> {
    let last = categories.split(';').map(str::trim).rfind(|category| {
        !category.is_empty() && !category.starts_with("X-") && !TOOLKITS.contains(category)
    })?;
    let chars: Vec<char> = last.chars().collect();
    let mut words = vec![String::new()];
    for (at, ch) in chars.iter().enumerate() {
        let prev = at.checked_sub(1).map(|prev| chars[prev]);
        let next = chars.get(at + 1);
        let starts_word = ch.is_uppercase()
            && prev.is_some_and(|prev| {
                prev.is_lowercase()
                    || (prev.is_uppercase() && next.is_some_and(|next| next.is_lowercase()))
            });
        if starts_word {
            words.push(String::new());
        }
        words.last_mut()?.push(*ch);
    }
    let words: Vec<String> = words
        .into_iter()
        .enumerate()
        .map(|(at, word)| {
            let acronym = word.chars().nth(1).is_some_and(char::is_uppercase);
            if at == 0 || acronym {
                word
            } else {
                word.to_lowercase()
            }
        })
        .collect();
    Some(words.join(" "))
}

pub fn file_icon(path: &Path) -> Option<PathBuf> {
    icons::file_icon(path)
}

pub fn recent_files(limit: usize) -> Vec<PathBuf> {
    recent::recent_files(limit)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_come_from_the_last_real_category_in_plain_words() {
        assert_eq!(
            kind("GNOME;GTK;System;TerminalEmulator;").as_deref(),
            Some("Terminal emulator")
        );
        assert_eq!(
            kind("GTK;Utility;TextEditor;").as_deref(),
            Some("Text editor")
        );
        assert_eq!(
            kind("Settings;X-Cinnamon-Settings-Panel;").as_deref(),
            Some("Settings")
        );
        assert_eq!(kind("Graphics;3DGraphics;").as_deref(), Some("3D graphics"));
        assert_eq!(kind("Development;IDE;").as_deref(), Some("IDE"));
        assert_eq!(kind("GTK;X-Foo;").as_deref(), None);
    }

    #[test]
    fn the_program_skips_env_and_its_variables() {
        let exec = |tokens: &[&str]| {
            tokens
                .iter()
                .map(|token| (*token).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(program(&exec(&["xed"])), Some("xed"));
        assert_eq!(
            program(&exec(&["env", "GDK_BACKEND=x11", "steam", "-silent"])),
            Some("steam")
        );
        assert_eq!(program(&[]), None);
    }

    #[test]
    fn sources_name_flatpak_and_snap_exports() {
        assert_eq!(
            source_of(Path::new(
                "/var/lib/flatpak/exports/share/applications/org.gimp.GIMP.desktop"
            )),
            Some("flatpak")
        );
        assert_eq!(
            source_of(Path::new(
                "/var/lib/snapd/desktop/applications/firefox_firefox.desktop"
            )),
            Some("snap")
        );
        assert_eq!(
            source_of(Path::new("/usr/share/applications/xed.desktop")),
            None
        );
    }
}
