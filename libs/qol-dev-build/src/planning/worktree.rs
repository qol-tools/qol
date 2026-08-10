use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn resolve_worktree_paths(
    dev_links: &HashMap<String, PathBuf>,
    branch: Option<&str>,
) -> HashMap<String, PathBuf> {
    dev_links
        .iter()
        .filter_map(|(id, path)| {
            resolve_plugin_worktree(path, branch).map(|resolved| (id.clone(), resolved))
        })
        .collect()
}

fn resolve_plugin_worktree(dev_link_path: &Path, branch: Option<&str>) -> Option<PathBuf> {
    let relative = match git_relative_path(dev_link_path) {
        Some(relative) => relative,
        None => return Some(dev_link_path.to_path_buf()),
    };

    if let Some(target_root) = match branch {
        Some(b) => find_git_worktree_by_branch(dev_link_path, b),
        None => find_git_worktree_base(dev_link_path),
    } {
        let candidate = target_root.join(&relative);
        if candidate.exists() {
            log::debug!(
                "[worktree] resolved {} -> {}",
                dev_link_path.display(),
                candidate.display()
            );
            return Some(candidate);
        }
    }

    if let Some(base_root) = find_git_worktree_base(dev_link_path) {
        let base_candidate = base_root.join(&relative);
        if base_candidate.exists() {
            log::debug!(
                "[worktree] {} missing in selection, falling back to {}",
                dev_link_path.display(),
                base_candidate.display()
            );
            return Some(base_candidate);
        }
    }

    log::debug!(
        "[worktree] dropping {}: not present in the active selection",
        dev_link_path.display()
    );
    None
}

fn git_relative_path(path: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel", "--show-prefix"])
        .current_dir(path)
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let relative = stdout.lines().nth(1).unwrap_or_default();
            Some(PathBuf::from(relative.trim()))
        }
        Ok(output) => {
            log::debug!(
                "[worktree] {} is not inside a git repo: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
            None
        }
        Err(error) => {
            log::warn!(
                "[worktree] could not run git in {}: {}",
                path.display(),
                error
            );
            None
        }
    }
}

fn run_git_worktree_list(repo_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_path)
        .output();
    match output {
        Ok(output) if output.status.success() => {
            Some(String::from_utf8_lossy(&output.stdout).into_owned())
        }
        Ok(output) => {
            log::warn!(
                "[worktree] git worktree list failed in {}: {}",
                repo_path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
            None
        }
        Err(error) => {
            log::warn!(
                "[worktree] could not run git worktree list in {}: {}",
                repo_path.display(),
                error
            );
            None
        }
    }
}

pub fn find_git_worktree_by_branch(repo_path: &Path, branch: &str) -> Option<PathBuf> {
    run_git_worktree_list(repo_path).and_then(|stdout| parse_worktree_for_branch(&stdout, branch))
}

pub fn find_git_worktree_base(repo_path: &Path) -> Option<PathBuf> {
    run_git_worktree_list(repo_path).and_then(|stdout| parse_worktree_base(&stdout))
}

fn parse_worktree_base(porcelain: &str) -> Option<PathBuf> {
    porcelain
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
}

fn parse_worktree_for_branch(porcelain: &str, target_branch: &str) -> Option<PathBuf> {
    let target_ref = format!("branch refs/heads/{}", target_branch);
    porcelain.split("\n\n").find_map(|block| {
        block
            .lines()
            .any(|line| line == target_ref)
            .then(|| {
                block
                    .lines()
                    .find_map(|line| line.strip_prefix("worktree "))
                    .map(PathBuf::from)
            })
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn canon(path: &Path) -> PathBuf {
        std::fs::canonicalize(path).expect("resolved path must exist on disk")
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_parse_worktree_matches_target_branch(
            path_a in "/[a-z0-9-]+(/[a-z0-9-]+)*",
            branch_a in "[a-z0-9-]+(/[a-z0-9-]+)*",
            path_b in "/[a-z0-9-]+(/[a-z0-9-]+)*",
            branch_b in "[a-z0-9-]+(/[a-z0-9-]+)*"
        ) {
            prop_assume!(branch_a != branch_b);

            let porcelain = format!(
                "worktree {path_a}\nHEAD abc123\nbranch refs/heads/{branch_a}\n\n\
                 worktree {path_b}\nHEAD def456\nbranch refs/heads/{branch_b}\n\n\
                 worktree /detached\nHEAD 789\ndetached\n\n"
            );

            prop_assert_eq!(
                parse_worktree_for_branch(&porcelain, &branch_a),
                Some(PathBuf::from(&path_a))
            );

            prop_assert_eq!(
                parse_worktree_for_branch(&porcelain, &branch_b),
                Some(PathBuf::from(&path_b))
            );

            prop_assert_eq!(
                parse_worktree_for_branch(&porcelain, "missing/branch/xyz"),
                None
            );

            prop_assert_eq!(
                parse_worktree_base(&porcelain),
                Some(PathBuf::from(&path_a))
            );
        }
    }

    #[test]
    fn resolution_prefers_selected_worktree_copy() {
        let repo = GitRepo::new();
        let base_plugin = repo.plugin(&repo.root, "plugin-a");
        let feat = repo.add_worktree("feat");
        let feat_plugin = repo.plugin(&feat, "plugin-a");

        let resolved = resolve_plugin_worktree(&base_plugin, Some("feat")).unwrap();
        assert_eq!(canon(&resolved), canon(&feat_plugin));
    }

    #[test]
    fn resolution_falls_back_to_main_when_worktree_copy_missing() {
        let repo = GitRepo::new();
        let base_plugin = repo.plugin(&repo.root, "plugin-a");
        let _feat = repo.add_worktree("feat");

        let resolved = resolve_plugin_worktree(&base_plugin, Some("feat")).unwrap();
        assert_eq!(canon(&resolved), canon(&base_plugin));
    }

    #[test]
    fn resolution_drops_worktree_link_outside_selection() {
        let repo = GitRepo::new();
        let feat = repo.add_worktree("feat");
        repo.add_worktree("other");
        let feat_plugin = repo.plugin(&feat, "plugin-a");

        assert_eq!(resolve_plugin_worktree(&feat_plugin, None), None);
        assert_eq!(resolve_plugin_worktree(&feat_plugin, Some("other")), None);
        let resolved =
            resolve_plugin_worktree(&feat_plugin, Some("feat")).expect("feat copy resolves");
        assert_eq!(canon(&resolved), canon(&feat_plugin));
    }

    #[test]
    fn resolve_worktree_paths_drops_detached_id_from_map() {
        let repo = GitRepo::new();
        let base_plugin = repo.plugin(&repo.root, "plugin-base");
        let feat = repo.add_worktree("feat");
        let feat_plugin = repo.plugin(&feat, "plugin-feat");
        let mut links = HashMap::new();
        links.insert("plugin-base".to_string(), base_plugin);
        links.insert("plugin-feat".to_string(), feat_plugin);

        let resolved = resolve_worktree_paths(&links, None);
        assert!(resolved.contains_key("plugin-base"));
        assert!(
            !resolved.contains_key("plugin-feat"),
            "detached ids must be absent from the resolved map"
        );
    }

    #[test]
    fn resolution_keeps_origin_for_links_outside_any_git_repo() {
        let temp = tempfile::tempdir().unwrap();
        let foreign = temp.path().join("standalone-plugin");
        std::fs::create_dir_all(&foreign).unwrap();

        assert_eq!(
            resolve_plugin_worktree(&foreign, Some("feat")),
            Some(foreign.clone())
        );
        assert_eq!(resolve_plugin_worktree(&foreign, None), Some(foreign));
    }

    #[test]
    fn resolution_keeps_standalone_repo_link_on_its_own_root() {
        let repo = GitRepo::new();
        let standalone = repo.plugin(&repo.root, "standalone");
        let resolved = resolve_plugin_worktree(&standalone, Some("unrelated")).unwrap();
        assert_eq!(canon(&resolved), canon(&standalone));
    }

    struct GitRepo {
        root: PathBuf,
        _temp: tempfile::TempDir,
    }

    impl GitRepo {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("repo");
            std::fs::create_dir_all(&root).unwrap();
            git(&root, &["init", "-q"]);
            git(&root, &["config", "user.email", "t@example.com"]);
            git(&root, &["config", "user.name", "t"]);
            git(&root, &["config", "commit.gpgsign", "false"]);
            git(&root, &["config", "core.hooksPath", "/dev/null"]);
            std::fs::write(root.join("README"), b"x").unwrap();
            git(&root, &["add", "."]);
            git(&root, &["commit", "-q", "-m", "init"]);
            Self { root, _temp: temp }
        }

        fn add_worktree(&self, branch: &str) -> PathBuf {
            let worktree = self.root.parent().unwrap().join(format!("wt-{branch}"));
            git(
                &self.root,
                &[
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    branch,
                    worktree.to_str().unwrap(),
                ],
            );
            worktree
        }

        fn plugin(&self, root: &Path, id: &str) -> PathBuf {
            let dir = root.join("plugins").join(id);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }
    }

    fn git(repo: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn parse_handles_edge_cases() {
        assert_eq!(parse_worktree_for_branch("", "main"), None);

        let detached_only = "worktree /detached\nHEAD 123\ndetached\n\n";
        assert_eq!(parse_worktree_for_branch(detached_only, "main"), None);

        let malformed_no_blank_lines = "worktree /a\nHEAD 1\nbranch refs/heads/main";
        assert_eq!(
            parse_worktree_for_branch(malformed_no_blank_lines, "main"),
            Some(PathBuf::from("/a"))
        );
    }
}
