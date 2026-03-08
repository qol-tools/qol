#![cfg(feature = "dev")]
use std::path::Path;

use super::super::types::WorktreeInfo;

pub(super) fn scan(manifest_dir: &Path) -> Vec<WorktreeInfo> {
    let mut results = vec![];

    if let Some(root) = find_dir_in_ancestors(manifest_dir, ".worktrees") {
        results.extend(collect_legacy(&root.join(".worktrees")));
    }

    if let Some(root) = find_dir_in_ancestors(manifest_dir, "worktrees") {
        results.extend(collect_centralized(&root.join("worktrees")));
    }

    results
}

fn find_dir_in_ancestors(start: &Path, dir_name: &str) -> Option<std::path::PathBuf> {
    start
        .ancestors()
        .find(|d| d.join(dir_name).is_dir())
        .map(|d| d.to_path_buf())
}

fn collect_legacy(wt_dir: &Path) -> Vec<WorktreeInfo> {
    collect_legacy_recursive(wt_dir, wt_dir, 0)
}

fn collect_legacy_recursive(root: &Path, dir: &Path, depth: u8) -> Vec<WorktreeInfo> {
    read_child_dirs(dir)
        .into_iter()
        .flat_map(|p| {
            if p.join("Cargo.toml").is_file() {
                let branch = p
                    .strip_prefix(root)
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                return vec![WorktreeInfo {
                    branch,
                    path: p.to_string_lossy().into_owned(),
                }];
            }
            if depth < 1 {
                return collect_legacy_recursive(root, &p, depth + 1);
            }
            vec![]
        })
        .collect()
}

fn collect_centralized(worktrees_dir: &Path) -> Vec<WorktreeInfo> {
    read_child_dirs(worktrees_dir)
        .into_iter()
        .filter_map(|feature_dir| {
            let branch = resolve_git_branch(&feature_dir)?;
            Some(WorktreeInfo {
                branch,
                path: feature_dir.to_string_lossy().into_owned(),
            })
        })
        .collect()
}

fn resolve_git_branch(feature_dir: &Path) -> Option<String> {
    let repo_dir = read_child_dirs(feature_dir)
        .into_iter()
        .find(|p| p.join("Cargo.toml").is_file())?;

    std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(&repo_dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_child_dirs(dir: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
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
