use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::FileEntry;

const MAX_FILES: usize = 8_000;
const MAX_DEPTH: usize = 6;

pub(crate) fn scan_files(roots: Vec<PathBuf>) -> Vec<FileEntry> {
    let mut files = Vec::new();
    for root in roots {
        if files.len() >= MAX_FILES {
            break;
        }
        collect_files(&root, 0, &mut files);
    }
    files.sort_by_cached_key(|file| file.name.to_lowercase());
    files
}

pub(crate) fn refresh_files(
    current: &[FileEntry],
    roots: &[PathBuf],
    changed_paths: &HashSet<PathBuf>,
) -> Vec<FileEntry> {
    let changed_paths = minimal_changed_paths(changed_paths);
    let changed_prefixes = changed_paths
        .iter()
        .map(PathBuf::as_path)
        .collect::<HashSet<_>>();
    let mut files = current
        .iter()
        .filter(|entry| {
            !entry
                .path
                .ancestors()
                .any(|path| changed_prefixes.contains(path))
        })
        .cloned()
        .collect::<Vec<_>>();

    for path in &changed_paths {
        if files.len() >= MAX_FILES {
            break;
        }
        let Some(root) = containing_root(roots, path) else {
            continue;
        };
        collect_changed_path(root, path, &mut files);
    }

    files.sort_by_cached_key(|file| (file.name.to_lowercase(), file.path.clone()));
    files.dedup_by(|left, right| left.path == right.path);
    files.truncate(MAX_FILES);
    files
}

fn minimal_changed_paths(changed_paths: &HashSet<PathBuf>) -> Vec<PathBuf> {
    let mut candidates = changed_paths.iter().cloned().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        left.components()
            .count()
            .cmp(&right.components().count())
            .then_with(|| left.cmp(right))
    });
    let mut selected = HashSet::new();
    let mut paths = Vec::new();
    for path in candidates {
        if path
            .ancestors()
            .skip(1)
            .any(|parent| selected.contains(parent))
        {
            continue;
        }
        selected.insert(path.clone());
        paths.push(path);
    }
    paths
}

fn containing_root<'a>(roots: &'a [PathBuf], path: &Path) -> Option<&'a Path> {
    roots
        .iter()
        .filter(|root| path.starts_with(root))
        .max_by_key(|root| root.as_os_str().len())
        .map(PathBuf::as_path)
}

fn collect_changed_path(root: &Path, path: &Path, out: &mut Vec<FileEntry>) {
    if !path_is_visible(root, path) {
        return;
    }
    let Ok(file_type) = fs::metadata(path).map(|metadata| metadata.file_type()) else {
        return;
    };
    if file_type.is_dir() {
        collect_files(path, 0, out);
        return;
    }
    if !file_type.is_file() {
        return;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return;
    };
    out.push(FileEntry {
        name: name.to_string(),
        path: path.to_path_buf(),
    });
}

fn path_is_visible(root: &Path, path: &Path) -> bool {
    path.strip_prefix(root).is_ok_and(|relative| {
        relative.components().all(|component| {
            !component
                .as_os_str()
                .to_str()
                .is_some_and(|name| name.starts_with('.'))
        })
    })
}

fn collect_files(dir: &Path, depth: usize, out: &mut Vec<FileEntry>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };

    for entry in read_dir.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            let is_hidden = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| name.starts_with('.'));
            if !is_hidden {
                collect_files(&path, depth + 1, out);
            }
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        out.push(FileEntry {
            name: name.to_string(),
            path,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::{minimal_changed_paths, refresh_files, FileEntry};
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    #[test]
    fn refresh_replaces_removed_and_created_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let removed = root.join("removed.txt");
        let created = root.join("created.txt");
        fs::write(&created, "fresh").unwrap();
        let current = vec![FileEntry {
            name: "removed.txt".to_string(),
            path: removed.clone(),
        }];
        let changed = HashSet::from([removed, created.clone()]);

        let refreshed = refresh_files(&current, &[root], &changed);

        assert_eq!(refreshed.len(), 1);
        assert_eq!(refreshed[0].path, created);
    }

    #[test]
    fn refresh_scans_created_directories_and_ignores_hidden_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let created = root.join("created");
        fs::create_dir(&created).unwrap();
        fs::write(created.join("visible.txt"), "visible").unwrap();
        fs::write(created.join(".hidden.txt"), "hidden").unwrap();
        let changed = HashSet::from([created]);

        let refreshed = refresh_files(&[], &[root], &changed);

        assert_eq!(refreshed.len(), 1);
        assert_eq!(refreshed[0].name, "visible.txt");
    }

    #[test]
    fn refresh_purges_removed_directories() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let removed = root.join("removed");
        let current = vec![FileEntry {
            name: "nested.txt".to_string(),
            path: removed.join("nested.txt"),
        }];
        let changed = HashSet::from([removed]);

        let refreshed = refresh_files(&current, &[root], &changed);

        assert!(refreshed.is_empty());
    }

    #[test]
    fn refresh_collapses_nested_change_paths() {
        let root = PathBuf::from("/files");
        let changed = HashSet::from([
            root.join("project"),
            root.join("project/src"),
            root.join("project/src/main.rs"),
            root.join("other.txt"),
        ]);

        let paths = minimal_changed_paths(&changed);

        assert_eq!(paths, vec![root.join("other.txt"), root.join("project")]);
    }

    #[test]
    fn unchanged_file_contents_do_not_change_the_index() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let path = root.join("stable.txt");
        fs::write(&path, "changed contents").unwrap();
        let current = vec![FileEntry {
            name: "stable.txt".to_string(),
            path: path.clone(),
        }];

        let refreshed = refresh_files(&current, &[root], &HashSet::from([path]));

        assert_eq!(refreshed, current);
    }
}
