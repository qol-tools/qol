mod recent;
mod version;

use std::path::{Path, PathBuf};

use qol_apps::shell_link::{LinkTarget, ShellLink};
use qol_watch::WatchRoot;

use super::super::details::{AppAbout, AppFace, Package};
use super::super::AppEntry;
use super::AppRoot;

const STORE_FOLDER: &str = "windowsapps";

pub fn cache_dir() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(std::env::temp_dir()))
}

pub fn app_roots() -> Vec<AppRoot> {
    qol_apps::start_menu::start_menu_roots()
}

pub fn scan_root(root: &AppRoot) -> Vec<AppEntry> {
    qol_apps::start_menu::scan_start_menu_root(root)
}

pub fn app_watch_root(root: &AppRoot) -> WatchRoot {
    WatchRoot::deep(root.path.clone())
}

pub fn app_change(_root: &AppRoot, path: &Path) -> Option<PathBuf> {
    match path.extension() {
        Some(_) if !qol_apps::start_menu::is_start_menu_shortcut(path) => None,
        _ => Some(path.to_path_buf()),
    }
}

pub fn file_watch_roots() -> Vec<PathBuf> {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    vec![
        PathBuf::from(format!("{home}\\Desktop")),
        PathBuf::from(format!("{home}\\Documents")),
        PathBuf::from(format!("{home}\\Downloads")),
    ]
}

struct Shortcut {
    link: Option<ShellLink>,
    target: Option<PathBuf>,
}

impl Shortcut {
    fn read(entry: &AppEntry) -> Self {
        let link = ShellLink::read(&entry.path);
        let target = link.as_ref().and_then(|link| {
            match link.target(&entry.path, |name| std::env::var(name).ok()) {
                LinkTarget::Path(path) => Some(path),
                LinkTarget::ShellFolder | LinkTarget::Advertised | LinkTarget::Unknown => None,
            }
        });
        Self { link, target }
    }

    fn binary(&self) -> Option<&Path> {
        self.target.as_deref().filter(|target| target.is_file())
    }

    fn comment(&self) -> Option<String> {
        self.link
            .as_ref()
            .and_then(|link| link.description.as_deref())
            .and_then(readable_comment)
    }

    fn advertised(&self) -> bool {
        self.link.as_ref().is_some_and(|link| link.advertised)
    }
}

pub fn app_face(entry: &AppEntry) -> AppFace {
    let shortcut = Shortcut::read(entry);
    let facts = shortcut.binary().map(version::read).unwrap_or_default();
    AppFace {
        icon: super::icon_cache::icon_path(&entry.path),
        description: shortcut
            .comment()
            .or_else(|| distinct(facts.description, &entry.name)),
    }
}

pub fn app_about(entry: &AppEntry) -> AppAbout {
    let shortcut = Shortcut::read(entry);
    let binary = shortcut.binary().map(Path::to_path_buf);
    let facts = binary.as_deref().map(version::read).unwrap_or_default();
    let arguments = shortcut
        .link
        .as_ref()
        .and_then(|link| link.arguments.as_deref());
    AppAbout {
        command: binary
            .as_deref()
            .and_then(|binary| command_line(binary, arguments)),
        source: source_of(binary.as_deref(), shortcut.advertised()),
        package: facts.product.map(|name| Package {
            name,
            version: facts.version,
            size: None,
            installed: None,
        }),
        developer: facts.company,
        binary,
        ..AppAbout::default()
    }
}

fn readable_comment(comment: &str) -> Option<String> {
    let comment = comment.trim();
    (!comment.is_empty() && !comment.starts_with('@')).then(|| comment.to_owned())
}

fn distinct(text: Option<String>, name: &str) -> Option<String> {
    text.filter(|text| !text.eq_ignore_ascii_case(name))
}

fn command_line(binary: &Path, arguments: Option<&str>) -> Option<String> {
    let arguments = arguments.map(str::trim).filter(|args| !args.is_empty())?;
    let program = binary.to_string_lossy();
    let program = if program.contains(' ') {
        format!("\"{program}\"")
    } else {
        program.into_owned()
    };
    Some(format!("{program} {arguments}"))
}

fn source_of(binary: Option<&Path>, advertised: bool) -> Option<&'static str> {
    if advertised {
        return Some("Windows Installer");
    }
    let binary = binary?.to_string_lossy().to_lowercase();
    let windows = std::env::var("SystemRoot")
        .ok()
        .map(|root| root.trim_end_matches('\\').to_lowercase());
    if binary
        .split('\\')
        .any(|component| component == STORE_FOLDER)
    {
        Some("Microsoft Store")
    } else if windows.is_some_and(|root| binary.starts_with(&format!("{root}\\"))) {
        Some("Windows")
    } else {
        None
    }
}

pub fn file_icon(_path: &Path) -> Option<PathBuf> {
    None
}

pub fn recent_files(limit: usize) -> Vec<PathBuf> {
    recent::recent_files(limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_comments_skip_blank_and_indirect_strings() {
        let cases = [
            ("Edit text files", Some("Edit text files")),
            ("  ", None),
            ("@%SystemRoot%\\system32\\shell32.dll,-22563", None),
        ];
        for (comment, expected) in cases {
            assert_eq!(readable_comment(comment).as_deref(), expected, "{comment}");
        }
    }

    #[test]
    fn commands_appear_only_when_the_shortcut_passes_arguments() {
        let cases = [
            (
                r"C:\Tools\code.exe",
                Some("--new-window"),
                Some(r"C:\Tools\code.exe --new-window"),
            ),
            (
                r"C:\Program Files\Foo\foo.exe",
                Some("-p work"),
                Some(r#""C:\Program Files\Foo\foo.exe" -p work"#),
            ),
            (r"C:\Tools\code.exe", Some("  "), None),
            (r"C:\Tools\code.exe", None, None),
        ];
        for (binary, arguments, expected) in cases {
            assert_eq!(
                command_line(Path::new(binary), arguments).as_deref(),
                expected,
                "{binary} {arguments:?}"
            );
        }
    }

    #[test]
    fn sources_name_the_store_and_windows_installer() {
        let cases = [
            (None, true, Some("Windows Installer")),
            (
                Some(r"C:\Program Files\WindowsApps\Foo_1.0_x64__abc\foo.exe"),
                false,
                Some("Microsoft Store"),
            ),
            (Some(r"C:\Program Files\Foo\foo.exe"), false, None),
            (None, false, None),
        ];
        for (binary, advertised, expected) in cases {
            assert_eq!(
                source_of(binary.map(Path::new), advertised),
                expected,
                "{binary:?}"
            );
        }
    }

    #[test]
    fn descriptions_that_repeat_the_name_are_dropped() {
        assert_eq!(distinct(Some("Notepad".into()), "notepad"), None);
        assert_eq!(
            distinct(Some("Text editor".into()), "Notepad").as_deref(),
            Some("Text editor")
        );
    }
}
