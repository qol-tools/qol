use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn resolve_worktree_paths(
    dev_links: &HashMap<String, PathBuf>,
    branch: Option<&str>,
) -> HashMap<String, PathBuf> {
    dev_links
        .iter()
        .map(|(id, path)| (id.clone(), resolve_plugin_worktree(path, branch)))
        .collect()
}

fn resolve_plugin_worktree(dev_link_path: &Path, branch: Option<&str>) -> PathBuf {
    let target_root = match branch {
        Some(b) => find_git_worktree_by_branch(dev_link_path, b),
        None => find_git_worktree_base(dev_link_path),
    };

    let Some(target_root) = target_root else {
        return dev_link_path.to_path_buf();
    };

    let current_root = git_toplevel(dev_link_path);
    let resolved = remap_to_worktree(dev_link_path, current_root.as_deref(), &target_root);

    if resolved == dev_link_path {
        log::debug!("[worktree] already on target: {}", dev_link_path.display());
        return resolved;
    }

    log::debug!(
        "[worktree] resolved {} -> {}",
        dev_link_path.display(),
        resolved.display()
    );
    resolved
}

fn remap_to_worktree(dev_link_path: &Path, current_root: Option<&Path>, target_root: &Path) -> PathBuf {
    let relative = current_root.and_then(|root| dev_link_path.strip_prefix(root).ok());
    match relative {
        Some(relative) => target_root.join(relative),
        None => target_root.to_path_buf(),
    }
}

fn git_toplevel(path: &Path) -> Option<PathBuf> {
    Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
}

fn run_git_worktree_list(repo_path: &Path) -> Option<String> {
    Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn find_git_worktree_by_branch(repo_path: &Path, branch: &str) -> Option<PathBuf> {
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
    fn remap_preserves_plugin_subpath_within_monorepo() {
        let cases = [
            (
                "/repo/plugins/plugin-x",
                Some("/repo"),
                "/wt/feat",
                "/wt/feat/plugins/plugin-x",
            ),
            (
                "/repo/plugins/plugin-x",
                Some("/repo"),
                "/repo",
                "/repo/plugins/plugin-x",
            ),
            (
                "/standalone-plugin",
                Some("/standalone-plugin"),
                "/standalone-plugin",
                "/standalone-plugin",
            ),
            (
                "/repo/plugins/plugin-x",
                None,
                "/wt/feat",
                "/wt/feat",
            ),
        ];
        for (dev_link, current_root, target_root, expected) in cases {
            let got = remap_to_worktree(
                Path::new(dev_link),
                current_root.map(Path::new),
                Path::new(target_root),
            );
            assert_eq!(
                got,
                PathBuf::from(expected),
                "dev_link={dev_link} current_root={current_root:?} target_root={target_root}"
            );
        }
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
