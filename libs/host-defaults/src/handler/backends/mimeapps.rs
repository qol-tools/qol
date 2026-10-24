use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};

use crate::xdg::XdgDirs;

pub const DEFAULT_GROUP: &str = "Default Applications";
pub const LIST_FILE: &str = "mimeapps.list";
const KEY_SEPARATOR: char = '|';

pub fn user_list_path(dirs: &XdgDirs) -> PathBuf {
    dirs.config_home.join(LIST_FILE)
}

pub fn user_list_paths_setting(dirs: &XdgDirs, desktops: &[String], mime: &str) -> Vec<PathBuf> {
    lists_in(&dirs.config_home, desktops)
        .into_iter()
        .filter(|path| {
            *path == user_list_path(dirs)
                || std::fs::read_to_string(path)
                    .is_ok_and(|text| get_value(&text, DEFAULT_GROUP, mime).is_some())
        })
        .collect()
}

pub fn key_for(path: &Path, mime: &str) -> String {
    format!("{}{KEY_SEPARATOR}{mime}", path.display())
}

pub fn split_key(key: &str) -> Result<(PathBuf, &str)> {
    let (path, mime) = key
        .rsplit_once(KEY_SEPARATOR)
        .ok_or_else(|| anyhow!("mimeapps key {key:?} is not <file>|<mime type>"))?;
    Ok((PathBuf::from(path), mime))
}

pub fn cascade_paths(dirs: &XdgDirs, desktops: &[String]) -> Vec<PathBuf> {
    std::iter::once(dirs.config_home.clone())
        .chain(dirs.config_dirs.iter().cloned())
        .chain(std::iter::once(dirs.data_home.join("applications")))
        .chain(dirs.data_dirs.iter().map(|dir| dir.join("applications")))
        .flat_map(|root| lists_in(&root, desktops))
        .collect()
}

fn lists_in(root: &Path, desktops: &[String]) -> Vec<PathBuf> {
    desktops
        .iter()
        .map(|desktop| root.join(format!("{desktop}-{LIST_FILE}")))
        .chain(std::iter::once(root.join(LIST_FILE)))
        .collect()
}

pub fn effective_default(
    texts: &[String],
    mime: &str,
    installed: impl Fn(&str) -> bool,
) -> Option<String> {
    texts
        .iter()
        .filter_map(|text| get_value(text, DEFAULT_GROUP, mime))
        .find_map(|value| {
            value
                .split(';')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .find(|id| installed(id))
                .map(str::to_string)
        })
}

pub fn get_value(text: &str, group: &str, key: &str) -> Option<String> {
    let lines = split_lines(text);
    let span = find_group(&lines, group)?;
    let index = find_key(&lines, span, key)?;
    key_value(&lines[index]).map(|(_, value)| value.to_string())
}

pub fn set_value(text: &str, group: &str, key: &str, value: &str) -> String {
    let mut lines = split_lines(text);
    let entry = format!("{key}={value}");
    let Some(span) = find_group(&lines, group) else {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(format!("[{group}]"));
        lines.push(entry);
        return join_lines(&lines);
    };
    match find_key(&lines, span, key) {
        Some(index) => lines[index] = entry,
        None => lines.insert(last_content(&lines, span) + 1, entry),
    }
    join_lines(&lines)
}

pub fn remove_value(text: &str, group: &str, key: &str) -> String {
    let mut lines = split_lines(text);
    let Some(span) = find_group(&lines, group) else {
        return text.to_string();
    };
    let Some(index) = find_key(&lines, span, key) else {
        return text.to_string();
    };
    lines.remove(index);
    let span = (span.0, span.1 - 1);
    if group_is_empty(&lines, span) {
        drop_group(&mut lines, span);
    }
    join_lines(&lines)
}

pub fn read_text(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

pub fn read_key(key: &str) -> Result<Option<String>> {
    let (path, mime) = split_key(key)?;
    Ok(read_text(&path)?.and_then(|text| get_value(&text, DEFAULT_GROUP, mime)))
}

pub fn write_key(key: &str, value: Option<&str>) -> Result<()> {
    let (path, mime) = split_key(key)?;
    let text = read_text(&path)?;
    let updated = match (value, text) {
        (Some(value), text) => set_value(&text.unwrap_or_default(), DEFAULT_GROUP, mime, value),
        (None, Some(text)) => {
            let updated = remove_value(&text, DEFAULT_GROUP, mime);
            if updated != text && has_no_entries(&updated) {
                return remove_list(&path);
            }
            updated
        }
        (None, None) => return Ok(()),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    qol_fs::atomic_write(&path, updated.as_bytes())
        .with_context(|| format!("failed to write {}", path.display()))
}

fn remove_list(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("failed to remove {}", path.display())),
    }
}

fn has_no_entries(text: &str) -> bool {
    text.lines()
        .all(|line| line.trim().is_empty() || group_name(line).is_some())
}

fn split_lines(text: &str) -> Vec<String> {
    text.lines().map(str::to_string).collect()
}

fn join_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    format!("{}\n", lines.join("\n"))
}

fn group_name(line: &str) -> Option<&str> {
    line.trim().strip_prefix('[')?.strip_suffix(']')
}

fn key_value(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (key, value) = line.split_once('=')?;
    Some((key.trim(), value.trim()))
}

fn find_group(lines: &[String], group: &str) -> Option<(usize, usize)> {
    let header = lines
        .iter()
        .position(|line| group_name(line) == Some(group))?;
    let end = lines[header + 1..]
        .iter()
        .position(|line| group_name(line).is_some())
        .map_or(lines.len(), |offset| header + 1 + offset);
    Some((header, end))
}

fn find_key(lines: &[String], span: (usize, usize), key: &str) -> Option<usize> {
    (span.0 + 1..span.1)
        .find(|&index| key_value(&lines[index]).is_some_and(|(found, _)| found == key))
}

fn last_content(lines: &[String], span: (usize, usize)) -> usize {
    (span.0..span.1)
        .rev()
        .find(|&index| !lines[index].trim().is_empty())
        .unwrap_or(span.0)
}

fn group_is_empty(lines: &[String], span: (usize, usize)) -> bool {
    lines[span.0 + 1..span.1]
        .iter()
        .all(|line| line.trim().is_empty())
}

fn drop_group(lines: &mut Vec<String>, span: (usize, usize)) {
    lines.drain(span.0..span.1);
    if span.0 < lines.len() {
        return;
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIME: &str = "inode/directory";

    #[test]
    fn get_reads_the_first_matching_key_in_the_group() {
        let text = "# comment\n[Added Associations]\ninode/directory=wrong.desktop\n\n[Default Applications]\nx-scheme-handler/http = firefox.desktop;\n inode/directory = nemo.desktop;thunar.desktop \n";
        let cases = [
            (DEFAULT_GROUP, MIME, Some("nemo.desktop;thunar.desktop")),
            (
                DEFAULT_GROUP,
                "x-scheme-handler/http",
                Some("firefox.desktop;"),
            ),
            ("Added Associations", MIME, Some("wrong.desktop")),
            (DEFAULT_GROUP, "text/html", None),
            ("Removed Associations", MIME, None),
        ];
        for (group, key, expected) in cases {
            assert_eq!(
                get_value(text, group, key).as_deref(),
                expected,
                "{group} / {key}"
            );
        }
    }

    #[test]
    fn get_ignores_commented_keys() {
        let text = "[Default Applications]\n#inode/directory=old.desktop\n";
        assert_eq!(get_value(text, DEFAULT_GROUP, MIME), None);
    }

    #[test]
    fn set_replaces_inserts_or_appends_and_keeps_everything_else() {
        let cases = [
            (
                "replace in place",
                "[Default Applications]\ninode/directory=nemo.desktop\ntext/html=a.desktop\n",
                "[Default Applications]\ninode/directory=qol-open.desktop\ntext/html=a.desktop\n",
            ),
            (
                "insert after the last key of the group",
                "[Default Applications]\ntext/html=a.desktop\n\n[Added Associations]\nx=y\n",
                "[Default Applications]\ntext/html=a.desktop\ninode/directory=qol-open.desktop\n\n[Added Associations]\nx=y\n",
            ),
            (
                "append a missing group",
                "[Added Associations]\nx=y\n",
                "[Added Associations]\nx=y\n\n[Default Applications]\ninode/directory=qol-open.desktop\n",
            ),
            (
                "empty file",
                "",
                "[Default Applications]\ninode/directory=qol-open.desktop\n",
            ),
            (
                "comments survive",
                "# top\n[Default Applications]\n# keep\ntext/html=a.desktop\n",
                "# top\n[Default Applications]\n# keep\ntext/html=a.desktop\ninode/directory=qol-open.desktop\n",
            ),
        ];
        for (label, text, expected) in cases {
            assert_eq!(
                set_value(text, DEFAULT_GROUP, MIME, "qol-open.desktop"),
                expected,
                "case: {label}"
            );
        }
    }

    #[test]
    fn remove_undoes_set_exactly() {
        let originals = [
            "[Default Applications]\ntext/html=a.desktop\n",
            "[Added Associations]\nx=y\n",
            "# only a comment\n",
            "[Default Applications]\ntext/html=a.desktop\n\n[Added Associations]\nx=y\n",
        ];
        for original in originals {
            let set = set_value(original, DEFAULT_GROUP, MIME, "qol-open.desktop");
            assert_eq!(
                remove_value(&set, DEFAULT_GROUP, MIME),
                original,
                "original: {original:?}"
            );
        }
    }

    #[test]
    fn remove_of_an_absent_key_returns_the_text_untouched() {
        let text = "[Default Applications]\ntext/html=a.desktop";
        assert_eq!(remove_value(text, DEFAULT_GROUP, MIME), text);
        assert_eq!(remove_value(text, "Added Associations", MIME), text);
    }

    #[test]
    fn remove_drops_an_emptied_group_in_the_middle() {
        let text = "[Added Associations]\nx=y\n\n[Default Applications]\ninode/directory=q.desktop\n\n[Removed Associations]\nz=w\n";
        assert_eq!(
            remove_value(text, DEFAULT_GROUP, MIME),
            "[Added Associations]\nx=y\n\n[Removed Associations]\nz=w\n"
        );
    }

    fn dirs() -> XdgDirs {
        XdgDirs {
            config_home: PathBuf::from("/home/u/.config"),
            config_dirs: vec![PathBuf::from("/etc/xdg")],
            data_home: PathBuf::from("/home/u/.local/share"),
            data_dirs: vec![PathBuf::from("/usr/share")],
        }
    }

    #[test]
    fn the_cascade_follows_the_spec_order() {
        let desktops = vec!["x-cinnamon".to_string(), "ubuntu".to_string()];
        let paths = cascade_paths(&dirs(), &desktops);
        assert_eq!(
            paths,
            [
                "/home/u/.config/x-cinnamon-mimeapps.list",
                "/home/u/.config/ubuntu-mimeapps.list",
                "/home/u/.config/mimeapps.list",
                "/etc/xdg/x-cinnamon-mimeapps.list",
                "/etc/xdg/ubuntu-mimeapps.list",
                "/etc/xdg/mimeapps.list",
                "/home/u/.local/share/applications/x-cinnamon-mimeapps.list",
                "/home/u/.local/share/applications/ubuntu-mimeapps.list",
                "/home/u/.local/share/applications/mimeapps.list",
                "/usr/share/applications/x-cinnamon-mimeapps.list",
                "/usr/share/applications/ubuntu-mimeapps.list",
                "/usr/share/applications/mimeapps.list",
            ]
            .map(PathBuf::from)
        );
    }

    #[test]
    fn the_cascade_without_a_desktop_lists_only_plain_files() {
        let paths = cascade_paths(&dirs(), &[]);
        assert_eq!(paths.len(), 4);
        assert!(paths.iter().all(|path| path.ends_with(LIST_FILE)));
    }

    #[test]
    fn the_effective_default_is_the_first_installed_entry_of_the_first_file_with_one() {
        let user =
            "[Default Applications]\ninode/directory=gone.desktop;qol-open.desktop;\n".to_string();
        let system =
            "[Default Applications]\ninode/directory=nemo.desktop;thunar.desktop\n".to_string();
        let other = "[Default Applications]\ntext/html=a.desktop\n".to_string();
        let cases = [
            (
                "skips an uninstalled and an excluded entry, falls through to the next file",
                vec![user.clone(), system.clone()],
                Some("nemo.desktop"),
            ),
            (
                "stays inside the first file when it has an installed entry",
                vec![system.clone(), user.clone()],
                Some("nemo.desktop"),
            ),
            (
                "a file without the key is skipped",
                vec![other.clone(), system.clone()],
                Some("nemo.desktop"),
            ),
            ("nothing lists it", vec![other], None),
            ("no files", vec![], None),
        ];
        for (label, texts, expected) in cases {
            let found = effective_default(&texts, MIME, |id| {
                id != "gone.desktop" && id != "qol-open.desktop"
            });
            assert_eq!(found.as_deref(), expected, "case: {label}");
        }
    }

    #[test]
    fn keys_split_on_the_last_separator() {
        let key = key_for(Path::new("/home/u/.config/mimeapps.list"), MIME);
        assert_eq!(key, "/home/u/.config/mimeapps.list|inode/directory");
        let (path, mime) = split_key(&key).unwrap();
        assert_eq!(path, PathBuf::from("/home/u/.config/mimeapps.list"));
        assert_eq!(mime, MIME);
        assert!(split_key("no-separator").is_err());
    }

    #[test]
    fn write_and_read_round_trip_through_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config").join(LIST_FILE);
        let key = key_for(&path, MIME);
        assert_eq!(read_key(&key).unwrap(), None);
        write_key(&key, None).unwrap();
        assert!(!path.exists());
        write_key(&key, Some("qol-open.desktop")).unwrap();
        assert_eq!(read_key(&key).unwrap().as_deref(), Some("qol-open.desktop"));
        write_key(&key, None).unwrap();
        assert_eq!(read_key(&key).unwrap(), None);
        assert!(!path.exists());
    }

    #[test]
    fn removing_the_last_entry_deletes_the_file_but_other_content_keeps_it() {
        let cases = [
            (
                "only our entry",
                "[Default Applications]\ninode/directory=q.desktop\n",
                false,
            ),
            (
                "empty sections around it",
                "[Added Associations]\n\n[Default Applications]\ninode/directory=q.desktop\n",
                false,
            ),
            (
                "another default stays",
                "[Default Applications]\ninode/directory=q.desktop\ntext/html=a.desktop\n",
                true,
            ),
            (
                "another section's entry stays",
                "[Added Associations]\nx=y\n\n[Default Applications]\ninode/directory=q.desktop\n",
                true,
            ),
            (
                "a comment stays",
                "# mine\n[Default Applications]\ninode/directory=q.desktop\n",
                true,
            ),
        ];
        for (label, text, survives) in cases {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(LIST_FILE);
            std::fs::write(&path, text).unwrap();
            write_key(&key_for(&path, MIME), None).unwrap();
            assert_eq!(path.exists(), survives, "case: {label}");
        }
    }

    #[test]
    fn removing_an_absent_key_leaves_an_existing_empty_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LIST_FILE);
        std::fs::write(&path, "").unwrap();
        write_key(&key_for(&path, MIME), None).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn has_no_entries_ignores_blank_lines_and_section_headers_only() {
        let cases = [
            ("", true),
            ("\n\n", true),
            ("[Default Applications]\n\n[Added Associations]\n", true),
            ("# note\n", false),
            ("[Default Applications]\nx=y\n", false),
        ];
        for (text, expected) in cases {
            assert_eq!(has_no_entries(text), expected, "text: {text:?}");
        }
    }
}
