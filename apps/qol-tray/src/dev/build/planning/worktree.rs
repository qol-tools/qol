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
    let match_path = match branch {
        Some(b) => find_git_worktree_by_branch(dev_link_path, b),
        None => find_git_worktree_base(dev_link_path),
    };

    let Some(match_path) = match_path else {
        return dev_link_path.to_path_buf();
    };

    if match_path == dev_link_path {
        log::debug!("[worktree] already on target: {}", dev_link_path.display());
        return dev_link_path.to_path_buf();
    }

    #[cfg(feature = "dev")]
    log::info!(
        "[worktree] resolved {} -> {}",
        dev_link_path.display(),
        match_path.display()
    );
    match_path
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

fn find_git_worktree_base(repo_path: &Path) -> Option<PathBuf> {
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

            // target matches exact nested path
            prop_assert_eq!(
                parse_worktree_for_branch(&porcelain, &branch_a),
                Some(PathBuf::from(&path_a))
            );

            prop_assert_eq!(
                parse_worktree_for_branch(&porcelain, &branch_b),
                Some(PathBuf::from(&path_b))
            );

            // missing branch safely returns None
            prop_assert_eq!(
                parse_worktree_for_branch(&porcelain, "missing/branch/xyz"),
                None
            );

            // base extraction securely picks the first absolute path
            prop_assert_eq!(
                parse_worktree_base(&porcelain),
                Some(PathBuf::from(&path_a))
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
