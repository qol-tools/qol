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
    let Some(root) = find_dir_in_ancestors(manifest_dir, "worktrees") else {
        return vec![];
    };
    collect_feature_grouped(&root.join("worktrees"), resolve_branch)
}

/// Feature-grouped layout: `worktrees/<branch-path>/<repo>/` must be a
/// git worktree with `Cargo.toml + .git`. `<repo>` may be any sibling
/// (qol-tray, qol-*, plugin-*), because the marker drives per-plugin
/// worktree resolution as well as qol-tray binary selection. Branches
/// with slashes nest naturally (`feat/x` → `worktrees/feat/x/<repo>/`).
fn collect_feature_grouped<F>(worktrees_dir: &Path, resolve_branch: F) -> Vec<WorktreeInfo>
where
    F: Fn(&Path) -> Option<String> + Copy,
{
    const MAX_DEPTH: u8 = 5;
    let mut out: Vec<WorktreeInfo> = vec![];
    for child in read_child_dirs(worktrees_dir) {
        walk_for_any_repo(&child, resolve_branch, &mut out, MAX_DEPTH);
    }
    out
}

fn walk_for_any_repo<F>(
    dir: &Path,
    resolve_branch: F,
    out: &mut Vec<WorktreeInfo>,
    depth_remaining: u8,
) where
    F: Fn(&Path) -> Option<String> + Copy,
{
    if dir.join("Cargo.toml").is_file() && dir.join(".git").exists() {
        if let Some(branch) = resolve_branch(dir) {
            if !out.iter().any(|w| w.branch == branch) {
                out.push(WorktreeInfo {
                    branch,
                    path: dir.to_string_lossy().into_owned(),
                });
            }
        }
        return;
    }
    if depth_remaining == 0 {
        return;
    }
    for child in read_child_dirs(dir) {
        walk_for_any_repo(&child, resolve_branch, out, depth_remaining - 1);
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
        .filter(|s| !s.is_empty() && s != "HEAD")
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
    fn scan_surfaces_plugin_only_feature() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let feature = tmp.path().join("worktrees").join("feat-config-contract-v1");
        create_git_worktree(&feature.join("plugin-window-actions"));
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert_eq!(result.len(), 1, "result: {:?}", result);
        assert_eq!(result[0].branch, "feat/config-contract-v1");
    }

    #[cfg(unix)]
    #[test]
    fn scan_dedupes_when_feature_contains_many_repos() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = create_manifest_dir(tmp.path(), "qol-tray");
        let feature = tmp.path().join("worktrees").join("feat-config-contract-v1");
        for repo in ["plugin-launcher", "plugin-alt-tab", "qol-tray"] {
            create_git_worktree(&feature.join(repo));
        }
        let result = scan_with_branch_resolver(&manifest_dir, fake_branch_resolver);

        assert_eq!(
            result.len(),
            1,
            "dedupe by branch must collapse multi-repo features: {:?}",
            result
        );
        assert_eq!(result[0].branch, "feat/config-contract-v1");
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
