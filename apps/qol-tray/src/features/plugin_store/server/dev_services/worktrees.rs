#![cfg(feature = "dev")]
use std::path::Path;

use super::super::types::WorktreeInfo;

pub(super) fn scan(manifest_dir: &Path) -> Vec<WorktreeInfo> {
    scan_with_branch_resolver(manifest_dir, resolve_git_branch)
}

fn scan_with_branch_resolver<F>(manifest_dir: &Path, resolve_branch: F) -> Vec<WorktreeInfo>
where
    F: Fn(&Path) -> Option<String> + Copy,
{
    let mut results = vec![];
    let repo_name = repo_name(manifest_dir);

    if let Some(root) = find_dir_in_ancestors(manifest_dir, ".worktrees") {
        results.extend(collect_legacy(&root.join(".worktrees")));
    }

    if let Some(root) = find_dir_in_ancestors(manifest_dir, "worktrees") {
        results.extend(collect_centralized(
            &root.join("worktrees"),
            repo_name,
            resolve_branch,
        ));
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

fn collect_centralized<F>(
    worktrees_dir: &Path,
    repo_name: Option<&str>,
    resolve_branch: F,
) -> Vec<WorktreeInfo>
where
    F: Fn(&Path) -> Option<String> + Copy,
{
    read_child_dirs(worktrees_dir)
        .into_iter()
        .filter_map(|feature_dir| {
            worktree_info_from_feature_dir(&feature_dir, repo_name, resolve_branch)
        })
        .collect()
}

fn worktree_info_from_feature_dir<F>(
    feature_dir: &Path,
    repo_name: Option<&str>,
    resolve_branch: F,
) -> Option<WorktreeInfo>
where
    F: Fn(&Path) -> Option<String> + Copy,
{
    let repo_dir = resolve_repo_dir(feature_dir, repo_name)?;
    let branch = resolve_branch(&repo_dir)?;
    Some(WorktreeInfo {
        branch,
        path: repo_dir.to_string_lossy().into_owned(),
    })
}

fn resolve_repo_dir(feature_dir: &Path, repo_name: Option<&str>) -> Option<std::path::PathBuf> {
    if let Some(repo_name) = repo_name {
        let repo_dir = feature_dir.join(repo_name);
        if repo_dir.join("Cargo.toml").is_file() {
            return Some(repo_dir);
        }
    }

    read_child_dirs(feature_dir)
        .into_iter()
        .find(|path| path.join("Cargo.toml").is_file())
}

fn resolve_git_branch(repo_dir: &Path) -> Option<String> {
    if !repo_dir.join(".git").exists() {
        return None;
    }

    std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(repo_dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
}

fn repo_name(manifest_dir: &Path) -> Option<&str> {
    manifest_dir.file_name().and_then(|name| name.to_str())
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
    use proptest::prelude::*;
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

    #[cfg(unix)]
    #[test]
    fn scan_finds_repo_specific_centralized_worktrees() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let feature = tmp.path().join("worktrees").join("feat-config-contract-v1");
        let tray_worktree = create_git_worktree(&feature.join("qol-tray"));
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert_eq!(result.len(), 1, "result: {:?}", result);
        assert_eq!(result[0].branch, "feat/config-contract-v1");
        assert_eq!(result[0].path, tray_worktree.to_string_lossy());
    }

    #[cfg(unix)]
    #[test]
    fn scan_falls_back_to_single_cargo_repo_when_repo_specific_dir_missing() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let feature = tmp.path().join("worktrees").join("feat-config-contract-v1");
        let fallback_repo = create_git_worktree(&feature.join("plugin-window-actions"));
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert_eq!(result.len(), 1, "result: {:?}", result);
        assert_eq!(result[0].path, fallback_repo.to_string_lossy());
    }

    #[cfg(unix)]
    #[test]
    fn scan_skips_centralized_feature_without_valid_repo_dir() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let feature = tmp.path().join("worktrees").join("feat-config-contract-v1");
        fs::create_dir_all(feature.join("plugin-window-actions")).unwrap();
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert!(result.is_empty(), "result: {:?}", result);
    }

    #[cfg(unix)]
    #[test]
    fn scan_skips_centralized_repo_without_git_dir() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let feature = tmp.path().join("worktrees").join("feat-config-contract-v1");
        create_worktree(&feature.join("qol-tray"), false);
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert!(result.is_empty(), "result: {:?}", result);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn prop_repo_name_uses_manifest_leaf(
            segments in prop::collection::vec("[a-z0-9_-]{1,12}", 1..6)
        ) {
            let mut path = std::path::PathBuf::new();
            for segment in &segments {
                path.push(segment);
            }

            prop_assert_eq!(repo_name(&path), segments.last().map(|segment| segment.as_str()));
        }
    }

    #[cfg(unix)]
    fn create_manifest_dir(root: &Path, repo_name: &str) -> std::path::PathBuf {
        let manifest_dir = root.join(repo_name);
        fs::create_dir_all(&manifest_dir).unwrap();
        fs::create_dir_all(root.join("worktrees")).unwrap();
        manifest_dir
    }

    #[cfg(unix)]
    fn create_git_worktree(path: &Path) -> std::path::PathBuf {
        create_worktree(path, true)
    }

    #[cfg(unix)]
    fn create_worktree(path: &Path, with_git_dir: bool) -> std::path::PathBuf {
        fs::create_dir_all(path).unwrap();
        fs::write(path.join("Cargo.toml"), "[package]").unwrap();
        if with_git_dir {
            fs::create_dir_all(path.join(".git")).unwrap();
        }
        path.to_path_buf()
    }

    #[cfg(unix)]
    fn fake_branch_resolver(repo_dir: &Path) -> Option<String> {
        repo_dir
            .join(".git")
            .exists()
            .then(|| "feat/config-contract-v1".to_string())
    }
}
