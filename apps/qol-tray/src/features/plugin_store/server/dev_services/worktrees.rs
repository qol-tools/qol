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
    let Some(repo_name) = repo_name(manifest_dir) else {
        return vec![];
    };
    let Some(root) = find_dir_in_ancestors(manifest_dir, "worktrees") else {
        return vec![];
    };
    collect_feature_grouped(&root.join("worktrees"), repo_name, resolve_branch)
}

/// Feature-grouped layout: `worktrees/<branch-path>/<repo_name>/` must
/// be a git worktree with `Cargo.toml + .git`. `<branch-path>` may
/// contain slashes - `feat/x` lives at `worktrees/feat/x/<repo_name>/`,
/// matching the git ref naturally. The walk descends until it finds the
/// `<repo_name>` anchor, then stops; foreign-repo worktrees never
/// surface because the anchor must literally be named `<repo_name>`.
fn collect_feature_grouped<F>(
    worktrees_dir: &Path,
    repo_name: &str,
    resolve_branch: F,
) -> Vec<WorktreeInfo>
where
    F: Fn(&Path) -> Option<String> + Copy,
{
    const MAX_DEPTH: u8 = 5;
    let mut out: Vec<WorktreeInfo> = vec![];
    for child in read_child_dirs(worktrees_dir) {
        walk_for_repo(&child, repo_name, resolve_branch, &mut out, MAX_DEPTH);
    }
    out
}

fn walk_for_repo<F>(
    dir: &Path,
    repo_name: &str,
    resolve_branch: F,
    out: &mut Vec<WorktreeInfo>,
    depth_remaining: u8,
) where
    F: Fn(&Path) -> Option<String> + Copy,
{
    let repo_dir = dir.join(repo_name);
    if repo_dir.join("Cargo.toml").is_file() && repo_dir.join(".git").exists() {
        if let Some(branch) = resolve_branch(&repo_dir) {
            if !out.iter().any(|w| w.branch == branch) {
                out.push(WorktreeInfo {
                    branch,
                    path: repo_dir.to_string_lossy().into_owned(),
                });
            }
        }
        return;
    }
    if depth_remaining == 0 {
        return;
    }
    for child in read_child_dirs(dir) {
        walk_for_repo(&child, repo_name, resolve_branch, out, depth_remaining - 1);
    }
}

fn find_dir_in_ancestors(start: &Path, dir_name: &str) -> Option<std::path::PathBuf> {
    start
        .ancestors()
        .find(|d| d.join(dir_name).is_dir())
        .map(|d| d.to_path_buf())
}

pub(super) fn resolve_git_branch(repo_dir: &Path) -> Option<String> {
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
    fn scan_returns_empty_when_no_worktrees_dir() {
        let tmp = TempDir::new().unwrap();
        let result = scan(tmp.path());
        assert!(result.is_empty(), "expected empty, got: {:?}", result);
    }

    #[cfg(unix)]
    #[test]
    fn scan_finds_feature_grouped_worktree() {
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
    fn scan_skips_feature_with_no_qol_tray_subdir() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let feature = tmp.path().join("worktrees").join("feat-config-contract-v1");
        create_git_worktree(&feature.join("plugin-window-actions"));
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert!(result.is_empty(), "result: {:?}", result);
    }

    #[cfg(unix)]
    #[test]
    fn scan_skips_flat_layout_without_repo_subdir() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let flat = tmp
            .path()
            .join("worktrees")
            .join("qol-tray-state-lifecycle");
        create_git_worktree(&flat);
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert!(
            result.is_empty(),
            "flat layout (cargo at feature root, no <feature>/qol-tray/) is not supported: {:?}",
            result
        );
    }

    #[cfg(unix)]
    #[test]
    fn scan_skips_repo_grouped_layout() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let grouping = tmp.path().join("worktrees").join("qol-tray");
        for branch in ["wasm", "tray-2-boot"] {
            create_git_worktree(&grouping.join(branch));
        }
        let result = scan_with_branch_resolver(&manifest_dir, leaf_branch_resolver);

        assert!(
            result.is_empty(),
            "worktrees/<repo>/<branch>/ is not the supported layout: {:?}",
            result
        );
    }

    #[cfg(unix)]
    #[test]
    fn scan_does_not_leak_foreign_repo_worktrees() {
        // The bug that motivated this convention: worktrees/qol-tray/qol-config/
        // is a qol-config worktree (perhaps on `main`), not qol-tray's. The strict
        // layout rejects it because there is no worktrees/qol-tray/qol-config/qol-tray/.
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let foreign = tmp.path().join("worktrees").join("qol-tray");
        create_git_worktree(&foreign.join("qol-config"));
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert!(
            result.is_empty(),
            "foreign-repo worktrees must not appear in the picker: {:?}",
            result
        );
    }

    #[cfg(unix)]
    #[test]
    fn scan_picks_qol_tray_when_feature_contains_many_repos() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let feature = tmp.path().join("worktrees").join("feat-config-contract-v1");
        for repo in ["plugin-launcher", "plugin-alt-tab", "qol-tray"] {
            create_git_worktree(&feature.join(repo));
        }
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert_eq!(result.len(), 1, "result: {:?}", result);
        assert!(
            result[0].path.ends_with("qol-tray"),
            "scan must resolve to qol-tray's own worktree, got: {}",
            result[0].path
        );
    }

    #[cfg(unix)]
    #[test]
    fn scan_finds_slash_layout_two_levels_deep() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let nested = tmp
            .path()
            .join("worktrees")
            .join("feat")
            .join("shortcuts-watcher");
        let tray_worktree = create_git_worktree(&nested.join("qol-tray"));
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert_eq!(result.len(), 1, "result: {:?}", result);
        assert_eq!(result[0].path, tray_worktree.to_string_lossy());
    }

    #[cfg(unix)]
    #[test]
    fn scan_finds_slash_layout_three_levels_deep() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let nested = tmp
            .path()
            .join("worktrees")
            .join("team")
            .join("a")
            .join("b");
        let tray_worktree = create_git_worktree(&nested.join("qol-tray"));
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert_eq!(result.len(), 1, "result: {:?}", result);
        assert_eq!(result[0].path, tray_worktree.to_string_lossy());
    }

    #[cfg(unix)]
    #[test]
    fn scan_stops_at_first_repo_anchor() {
        // worktrees/feat/qol-tray exists AND worktrees/feat/x/qol-tray exists.
        // The outer anchor wins; the deeper one is shadowed. This prevents an
        // accidentally-nested anchor from doubling entries.
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let feat = tmp.path().join("worktrees").join("feat");
        let outer = create_git_worktree(&feat.join("qol-tray"));
        create_git_worktree(&feat.join("x").join("qol-tray"));
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert_eq!(result.len(), 1, "result: {:?}", result);
        assert_eq!(result[0].path, outer.to_string_lossy());
    }

    #[cfg(unix)]
    #[test]
    fn scan_skips_repo_dir_without_git() {
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

    #[cfg(unix)]
    fn leaf_branch_resolver(repo_dir: &Path) -> Option<String> {
        if !repo_dir.join(".git").exists() {
            return None;
        }
        repo_dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    }
}
