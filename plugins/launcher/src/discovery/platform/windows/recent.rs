use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use qol_apps::shell_link::{LinkTarget, ShellLink};

const RECENT_DIR: &str = r"Microsoft\Windows\Recent";

pub(super) fn recent_files(limit: usize) -> Vec<PathBuf> {
    let Some(dir) = std::env::var_os("APPDATA")
        .filter(|value| !value.is_empty())
        .map(|appdata| PathBuf::from(appdata).join(RECENT_DIR))
    else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let links = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| qol_apps::start_menu::is_start_menu_shortcut(path))
        .filter_map(|path| {
            let modified = std::fs::metadata(&path).and_then(|meta| meta.modified());
            Some((modified.ok()?, path))
        })
        .collect();
    newest_targets(links, limit, link_target, Path::is_file)
}

fn link_target(link: &Path) -> Option<PathBuf> {
    match ShellLink::read(link)?.target(link, |name| std::env::var(name).ok()) {
        LinkTarget::Path(path) => Some(path),
        LinkTarget::ShellFolder | LinkTarget::Advertised | LinkTarget::Unknown => None,
    }
}

fn newest_targets(
    mut links: Vec<(SystemTime, PathBuf)>,
    limit: usize,
    resolve: impl Fn(&Path) -> Option<PathBuf>,
    is_file: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    links.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    let mut seen = HashSet::new();
    links
        .iter()
        .filter_map(|(_, link)| resolve(link))
        .filter(|target| is_file(target))
        .filter(|target| seen.insert(target.to_string_lossy().to_lowercase()))
        .take(limit)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    #[test]
    fn newest_existing_files_come_first_once_each() {
        let at = |secs| UNIX_EPOCH + Duration::from_secs(secs);
        let links = vec![
            (at(1), PathBuf::from("old.lnk")),
            (at(5), PathBuf::from("newest.lnk")),
            (at(3), PathBuf::from("folder.lnk")),
            (at(4), PathBuf::from("again.lnk")),
            (at(2), PathBuf::from("gone.lnk")),
        ];
        let resolve = |link: &Path| match link.to_str()? {
            "old.lnk" => Some(PathBuf::from(r"C:\docs\old.txt")),
            "newest.lnk" => Some(PathBuf::from(r"C:\docs\report.pdf")),
            "folder.lnk" => Some(PathBuf::from(r"C:\docs")),
            "again.lnk" => Some(PathBuf::from(r"C:\DOCS\Report.pdf")),
            "gone.lnk" => Some(PathBuf::from(r"C:\docs\deleted.txt")),
            _ => None,
        };
        let is_file = |path: &Path| {
            let name = path.to_string_lossy().to_lowercase();
            name.ends_with(".pdf") || name.ends_with("old.txt")
        };
        let cases = [
            (
                10,
                vec![
                    PathBuf::from(r"C:\docs\report.pdf"),
                    PathBuf::from(r"C:\docs\old.txt"),
                ],
            ),
            (1, vec![PathBuf::from(r"C:\docs\report.pdf")]),
            (0, Vec::new()),
        ];
        for (limit, expected) in cases {
            assert_eq!(
                newest_targets(links.clone(), limit, resolve, is_file),
                expected,
                "limit={limit}"
            );
        }
    }
}
