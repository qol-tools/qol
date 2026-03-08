#![cfg(feature = "dev")]
use std::path::Path;

use super::super::types::WorktreeInfo;

pub(super) fn scan(manifest_dir: &Path) -> Vec<WorktreeInfo> {
    let Some(root) = find_worktrees_root(manifest_dir) else {
        return vec![];
    };
    let wt_dir = root.join(".worktrees");
    collect(&wt_dir, &wt_dir, 0)
}

fn find_worktrees_root(start: &Path) -> Option<std::path::PathBuf> {
    start
        .ancestors()
        .find(|d| d.join(".worktrees").is_dir())
        .map(|d| d.to_path_buf())
}

fn collect(root: &Path, dir: &Path, depth: u8) -> Vec<WorktreeInfo> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .flat_map(|p| process_entry(root, &p, depth))
                .collect()
        })
        .unwrap_or_default()
}

fn process_entry(root: &Path, path: &Path, depth: u8) -> Vec<WorktreeInfo> {
    if path.join("Cargo.toml").is_file() {
        let branch = path
            .strip_prefix(root)
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        return vec![WorktreeInfo {
            branch,
            path: path.to_string_lossy().into_owned(),
        }];
    }

    if depth < 1 {
        return collect(root, path, depth + 1);
    }

    vec![]
}

#[cfg(all(test, feature = "dev"))]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn scan_finds_two_level_worktrees() {
        let tmp = TempDir::new().unwrap();
        let wt = tmp
            .path()
            .join(".worktrees")
            .join("feat")
            .join("my-feature");
        fs::create_dir_all(&wt).unwrap();
        fs::write(wt.join("Cargo.toml"), "[package]").unwrap();

        let result = scan(tmp.path());
        assert_eq!(result.len(), 1, "should find one worktree");
        assert_eq!(
            result[0].branch, "feat/my-feature",
            "branch: {}",
            result[0].branch
        );
        assert_eq!(
            result[0].path,
            wt.to_string_lossy().as_ref(),
            "path: {}",
            result[0].path
        );
    }

    #[test]
    fn scan_ignores_dirs_without_cargo_toml() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".worktrees").join("feat").join("no-cargo")).unwrap();

        let result = scan(tmp.path());
        assert!(result.is_empty(), "expected empty, got: {:?}", result);
    }

    #[test]
    fn scan_returns_empty_when_no_worktrees_dir() {
        let tmp = TempDir::new().unwrap();
        let result = scan(tmp.path());
        assert!(result.is_empty(), "expected empty, got: {:?}", result);
    }

    #[test]
    fn scan_finds_multiple_worktrees() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join(".worktrees");
        for branch in ["feat/foo", "feat/bar", "refactor/baz"] {
            let wt = root.join(std::path::PathBuf::from(branch));
            fs::create_dir_all(&wt).unwrap();
            fs::write(wt.join("Cargo.toml"), "[package]").unwrap();
        }

        let mut result = scan(tmp.path());
        result.sort_by(|a, b| a.branch.cmp(&b.branch));
        let branches: Vec<&str> = result.iter().map(|w| w.branch.as_str()).collect();
        assert_eq!(
            branches,
            ["feat/bar", "feat/foo", "refactor/baz"],
            "branches: {:?}",
            branches
        );
    }
}
