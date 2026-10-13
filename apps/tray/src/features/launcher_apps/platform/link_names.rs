use super::super::LauncherEntry;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

pub(super) const LINK_EXTENSION: &str = "lnk";
const INVALID_NAME_CHARS: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
const FALLBACK_BASE: &str = "QoL";
const ID_HASH_BYTES: usize = 4;

pub(super) fn link_names(entries: &[LauncherEntry]) -> Vec<String> {
    let bases: Vec<Option<String>> = entries
        .iter()
        .map(|entry| link_base(&entry.display_name))
        .collect();
    let mut counts = HashMap::new();
    for base in bases.iter().flatten() {
        *counts.entry(base.to_lowercase()).or_insert(0usize) += 1;
    }
    entries
        .iter()
        .zip(&bases)
        .map(|(entry, base)| match base {
            Some(base) if counts[&base.to_lowercase()] == 1 => format!("{base}.{LINK_EXTENSION}"),
            Some(base) => format!("{base} ({}).{LINK_EXTENSION}", id_hash(&entry.file_stem)),
            None => format!(
                "{} ({}).{LINK_EXTENSION}",
                link_base(&entry.file_stem).unwrap_or_else(|| FALLBACK_BASE.to_string()),
                id_hash(&entry.file_stem)
            ),
        })
        .collect()
}

pub(super) fn link_path(dir: &Path, name: &str) -> Option<PathBuf> {
    let link = dir.join(name);
    (link.parent() == Some(dir) && link.file_name() == Some(OsStr::new(name))).then_some(link)
}

fn link_base(text: &str) -> Option<String> {
    let cleaned: String = text
        .chars()
        .map(|ch| {
            if ch.is_control() || INVALID_NAME_CHARS.contains(&ch) {
                ' '
            } else {
                ch
            }
        })
        .collect();
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = collapsed.trim_end_matches(['.', ' ']);
    if name.is_empty() || is_reserved_device_name(name) {
        return None;
    }
    Some(name.to_string())
}

fn id_hash(id: &str) -> String {
    Sha256::digest(id.as_bytes())[..ID_HASH_BYTES]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_reserved_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    match stem.as_bytes() {
        b"CON" | b"PRN" | b"AUX" | b"NUL" => true,
        [b'C', b'O', b'M', digit] | [b'L', b'P', b'T', digit] => digit.is_ascii_digit(),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::icon::LauncherIcon;
    use super::*;
    use qol_theme::Mark;

    fn mark(file_stem: &str, display_name: &str) -> LauncherEntry {
        LauncherEntry {
            file_stem: file_stem.into(),
            display_name: display_name.into(),
            description: format!("QoL: {display_name}"),
            bundle_id: String::new(),
            exec_args: vec!["exec".into(), "shortcut".into(), file_stem.into()],
            shortcut_action: None,
            icon: LauncherIcon::Mark(Mark::Shortcuts),
        }
    }

    #[test]
    fn link_names_are_valid_windows_file_names() {
        let entries = [
            mark("command-add", "QoL \u{203a} Add Shortcut"),
            mark("shortcut-a", "Docs: a/b \\ c?"),
            mark("shortcut-b", "  Trailing dots...  "),
            mark("shortcut-c", "Aux"),
            mark("shortcut-d", "com1.backup"),
            mark("shortcut-e", "Same"),
            mark("shortcut-f", "same"),
            mark("shortcut-g", "<>|*"),
            mark("shortcut-h", "Line\nbreak"),
        ];
        let hashed = |base: &str, file_stem: &str| format!("{base} ({}).lnk", id_hash(file_stem));

        assert_eq!(
            link_names(&entries),
            [
                "QoL \u{203a} Add Shortcut.lnk".to_string(),
                "Docs a b c.lnk".to_string(),
                "Trailing dots.lnk".to_string(),
                hashed("shortcut-c", "shortcut-c"),
                hashed("shortcut-d", "shortcut-d"),
                hashed("Same", "shortcut-e"),
                hashed("same", "shortcut-f"),
                hashed("shortcut-g", "shortcut-g"),
                "Line break.lnk".to_string(),
            ]
        );
    }

    #[test]
    fn id_hash_is_a_short_stable_digest_of_the_id() {
        assert_eq!(id_hash("shortcut-c"), "606a3d97");
        assert_eq!(id_hash("shortcut-a/../../Startup/qol-probe"), "a416e315");
    }

    #[test]
    fn links_never_leave_the_qol_folder_whatever_the_id() {
        let dir = Path::new("Start Menu").join("Programs").join("QoL");
        let hostile = "shortcut-a/../../Startup/qol-probe";
        let entries = [
            mark("shortcut-dup", "Dup"),
            mark(hostile, "Dup"),
            mark(hostile, ""),
            mark(hostile, "Aux"),
            mark("..", ""),
            mark(r"..\..\Startup\x", "  "),
            mark("C:x", "..."),
        ];

        for name in link_names(&entries) {
            let link = link_path(&dir, &name).unwrap_or_else(|| panic!("refused {name:?}"));
            assert_eq!(link.parent(), Some(dir.as_path()), "{name:?}");
            assert!(!name.contains(['/', '\\', ':']), "{name:?}");
        }
        assert_eq!(
            link_names(&entries)[1],
            "Dup (a416e315).lnk",
            "a colliding name is suffixed with the id hash, never the raw id"
        );
    }

    #[test]
    fn link_path_refuses_names_that_leave_the_folder() {
        let dir = Path::new("Start Menu").join("Programs").join("QoL");
        for name in ["a/../b.lnk", "sub/x.lnk", "..", "../x.lnk", ""] {
            assert_eq!(link_path(&dir, name), None, "{name:?}");
        }
        assert_eq!(link_path(&dir, "x.lnk"), Some(dir.join("x.lnk")));
    }
}
