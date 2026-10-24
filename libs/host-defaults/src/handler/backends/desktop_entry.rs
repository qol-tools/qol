use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::role::Role;
use crate::xdg::XdgDirs;

pub const OPENER_ID: &str = "qol-open.desktop";
const ENTRY_GROUP: &str = "Desktop Entry";

pub fn opener_path(dirs: &XdgDirs) -> PathBuf {
    dirs.data_home.join("applications").join(OPENER_ID)
}

pub fn render_opener(exec: &Path) -> String {
    let role = Role::FolderOpen;
    format!(
        "[Desktop Entry]\nType=Application\nName=qol\nNoDisplay=true\nMimeType=inode/directory;\nExec={} defaults open {} %U\n",
        exec_argument(&exec.to_string_lossy()),
        role.id(),
    )
}

pub fn exec_argument(argument: &str) -> String {
    let plain = !argument.is_empty()
        && argument
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-./:@+=,".contains(c));
    if plain {
        return argument.to_string();
    }
    let escaped = argument.chars().fold(String::new(), |mut out, c| {
        match c {
            '"' | '`' | '$' => out.push_str("\\\\"),
            '\\' => out.push_str("\\\\\\"),
            '%' => out.push('%'),
            _ => {}
        }
        out.push(c);
        out
    });
    format!("\"{escaped}\"")
}

pub fn parse_name(text: &str) -> Option<String> {
    parse_field(text, "Name")
}

pub fn parse_exec(text: &str) -> Option<String> {
    parse_field(text, "Exec")
}

pub fn program_stem(exec: &str) -> Option<String> {
    let mut tokens = exec.split_whitespace().map(|token| token.trim_matches('"'));
    let program = program_token(&mut tokens)?;
    let stem = basename(program);
    if stem == "flatpak" {
        return flatpak_stem(tokens);
    }
    if stem.is_empty() || stem.starts_with('%') {
        return None;
    }
    Some(stem.to_string())
}

pub fn owner_label(name: Option<String>, stem: Option<String>) -> Option<String> {
    match (name, stem) {
        (Some(name), Some(stem)) if !contains_ignoring_case(&name, &stem) => {
            Some(title_case(&stem))
        }
        (Some(name), _) => Some(name),
        (None, Some(stem)) => Some(title_case(&stem)),
        (None, None) => None,
    }
}

pub fn entry_owner(dirs: &XdgDirs, id: &str) -> Option<String> {
    let text = std::fs::read_to_string(find_entry(dirs, id)?).ok()?;
    owner_label(
        parse_name(&text),
        parse_exec(&text).and_then(|exec| program_stem(&exec)),
    )
}

fn program_token<'a>(tokens: &mut impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let first = tokens.next()?;
    if basename(first) != "env" {
        return Some(first);
    }
    tokens.find(|token| !token.starts_with('-') && !token.contains('='))
}

fn flatpak_stem<'a>(mut tokens: impl Iterator<Item = &'a str>) -> Option<String> {
    if tokens.next()? != "run" {
        return None;
    }
    let id = tokens.find(|token| !token.starts_with('-'))?;
    let segment = id
        .rsplit('.')
        .next()
        .filter(|segment| !segment.is_empty())?;
    Some(segment.to_string())
}

fn basename(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
}

fn contains_ignoring_case(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

fn title_case(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn parse_field(text: &str, field: &str) -> Option<String> {
    let mut in_entry = false;
    for line in text.lines().map(str::trim) {
        if let Some(group) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            in_entry = group == ENTRY_GROUP;
            continue;
        }
        if !in_entry {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == field && !value.trim().is_empty() {
            return Some(value.trim().to_string());
        }
    }
    None
}

pub fn find_entry(dirs: &XdgDirs, id: &str) -> Option<PathBuf> {
    std::iter::once(dirs.data_home.join("applications"))
        .chain(dirs.data_dirs.iter().map(|dir| dir.join("applications")))
        .map(|root| root.join(id))
        .find(|path| path.is_file())
}

pub fn read_key(key: &str) -> Result<Option<String>> {
    match std::fs::read_to_string(key) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {key}")),
    }
}

pub fn write_key(key: &str, value: Option<&str>) -> Result<()> {
    let path = Path::new(key);
    let Some(content) = value else {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error).with_context(|| format!("failed to remove {key}")),
        };
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    qol_fs::atomic_write(path, content.as_bytes()).with_context(|| format!("failed to write {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_opener_is_a_hidden_folder_handler_that_forwards_to_the_tray() {
        let text = render_opener(Path::new("/opt/qol/qol-tray"));
        assert_eq!(
            text,
            "[Desktop Entry]\nType=Application\nName=qol\nNoDisplay=true\nMimeType=inode/directory;\nExec=/opt/qol/qol-tray defaults open folder.open %U\n"
        );
    }

    #[test]
    fn exec_arguments_are_quoted_only_when_needed() {
        let cases = [
            ("/opt/qol/qol-tray", "/opt/qol/qol-tray"),
            ("/home/a b/qol", "\"/home/a b/qol\""),
            ("/home/$user/qol", "\"/home/\\\\$user/qol\""),
            ("/odd\"name", "\"/odd\\\\\"name\""),
            ("/100%/qol", "\"/100%%/qol\""),
            ("/back\\slash", "\"/back\\\\\\\\slash\""),
            ("", "\"\""),
        ];
        for (argument, expected) in cases {
            assert_eq!(exec_argument(argument), expected, "argument: {argument:?}");
        }
    }

    #[test]
    fn the_name_comes_from_the_desktop_entry_group_only() {
        let cases = [
            (
                "plain",
                "[Desktop Entry]\nName=Files\nName[de]=Dateien\nExec=nemo\n",
                Some("Files"),
            ),
            (
                "other group first",
                "[Desktop Action new-window]\nName=New Window\n\n[Desktop Entry]\nName=Nemo\n",
                Some("Nemo"),
            ),
            (
                "localized only",
                "[Desktop Entry]\nName[de]=Dateien\n",
                None,
            ),
            (
                "spaces around the equals sign",
                "[Desktop Entry]\nName = Dolphin \n",
                Some("Dolphin"),
            ),
            ("no entry group", "Name=Loose\n", None),
        ];
        for (label, text, expected) in cases {
            assert_eq!(parse_name(text).as_deref(), expected, "case: {label}");
        }
    }

    fn dirs(root: &Path) -> XdgDirs {
        XdgDirs {
            config_home: root.join("config"),
            config_dirs: Vec::new(),
            data_home: root.join("data"),
            data_dirs: vec![root.join("system")],
        }
    }

    #[test]
    fn entries_are_found_user_first_then_system() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        let user = dirs.data_home.join("applications");
        let system = dirs.data_dirs[0].join("applications");
        std::fs::create_dir_all(&user).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(
            system.join("nemo.desktop"),
            "[Desktop Entry]\nName=System Nemo\n",
        )
        .unwrap();
        std::fs::write(
            system.join("only-system.desktop"),
            "[Desktop Entry]\nName=Sys\n",
        )
        .unwrap();
        assert_eq!(
            entry_owner(&dirs, "nemo.desktop").as_deref(),
            Some("System Nemo")
        );
        std::fs::write(
            user.join("nemo.desktop"),
            "[Desktop Entry]\nName=User Nemo\n",
        )
        .unwrap();
        assert_eq!(
            entry_owner(&dirs, "nemo.desktop").as_deref(),
            Some("User Nemo")
        );
        assert_eq!(
            entry_owner(&dirs, "only-system.desktop").as_deref(),
            Some("Sys")
        );
        assert_eq!(entry_owner(&dirs, "missing.desktop"), None);
        assert!(find_entry(&dirs, "missing.desktop").is_none());
    }

    #[test]
    fn the_program_stem_skips_args_field_codes_env_and_flatpak_wrappers() {
        let cases = [
            ("nemo %U", Some("nemo")),
            ("/usr/bin/nautilus --new-window %U", Some("nautilus")),
            ("\"/opt/my app/dolphin\" %u", Some("dolphin")),
            ("env GDK_BACKEND=x11 -u FOO thunar %F", Some("thunar")),
            (
                "/usr/bin/flatpak run --branch=stable --arch=x86_64 --command=nautilus org.gnome.Nautilus @@u %U @@",
                Some("Nautilus"),
            ),
            ("flatpak info org.gnome.Nautilus", None),
            ("%U", None),
            ("", None),
        ];
        for (exec, expected) in cases {
            assert_eq!(program_stem(exec).as_deref(), expected, "exec: {exec:?}");
        }
    }

    #[test]
    fn the_owner_keeps_a_name_that_holds_the_program_and_otherwise_uses_the_program() {
        let cases = [
            ("nemo.desktop", "Files", "nemo %U", Some("Nemo")),
            (
                "org.gnome.Nautilus.desktop",
                "Files",
                "nautilus --new-window %U",
                Some("Nautilus"),
            ),
            (
                "org.kde.dolphin.desktop",
                "Dolphin",
                "dolphin %u",
                Some("Dolphin"),
            ),
            (
                "pcmanfm.desktop",
                "File Manager PCManFM",
                "pcmanfm %U",
                Some("File Manager PCManFM"),
            ),
            (
                "flatpak.desktop",
                "Files",
                "flatpak run org.gnome.Nautilus %U",
                Some("Nautilus"),
            ),
            ("noexec.desktop", "Files", "", Some("Files")),
        ];
        for (id, name, exec, expected) in cases {
            let root = tempfile::tempdir().unwrap();
            let dirs = dirs(root.path());
            let applications = dirs.data_dirs[0].join("applications");
            std::fs::create_dir_all(&applications).unwrap();
            let exec_line = if exec.is_empty() {
                String::new()
            } else {
                format!("Exec={exec}\n")
            };
            std::fs::write(
                applications.join(id),
                format!("[Desktop Entry]\nName={name}\n{exec_line}"),
            )
            .unwrap();
            assert_eq!(entry_owner(&dirs, id).as_deref(), expected, "id: {id}");
        }
    }

    #[test]
    fn the_owner_falls_back_to_the_program_when_the_name_is_missing() {
        assert_eq!(
            owner_label(None, Some("nemo".to_string())).as_deref(),
            Some("Nemo")
        );
        assert_eq!(owner_label(None, None), None);
    }

    #[test]
    fn the_opener_path_lives_in_the_user_applications_dir() {
        let root = tempfile::tempdir().unwrap();
        let dirs = dirs(root.path());
        assert_eq!(
            opener_path(&dirs),
            root.path()
                .join("data")
                .join("applications")
                .join(OPENER_ID)
        );
    }

    #[test]
    fn write_read_and_delete_round_trip_through_a_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("nested").join(OPENER_ID);
        let key = path.to_string_lossy().into_owned();
        assert_eq!(read_key(&key).unwrap(), None);
        write_key(&key, None).unwrap();
        write_key(&key, Some("content\n")).unwrap();
        assert_eq!(read_key(&key).unwrap().as_deref(), Some("content\n"));
        write_key(&key, None).unwrap();
        assert_eq!(read_key(&key).unwrap(), None);
        assert!(!path.exists());
    }
}
