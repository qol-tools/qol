use super::command::{run_git, run_git_checked};
use anyhow::Result;
use std::path::Path;
use std::time::Duration;

pub(super) async fn clone_plugin_repo(repo_url: &str, staging_dir: &Path) -> Result<()> {
    let staging_str = path_utf8(staging_dir)?;
    log::info!("Cloning plugin from {} to {:?}", repo_url, staging_dir);
    run_git_checked(["clone", repo_url, staging_str], None, "clone").await?;
    Ok(())
}

pub(super) async fn prepare_update_repo(staging_dir: &Path, repo_url: &str) -> Result<()> {
    clone_plugin_repo(repo_url, staging_dir).await?;
    let branch = get_default_branch(staging_dir).await?;
    reset_to_origin_head(staging_dir, &branch).await
}

fn path_utf8(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| anyhow::anyhow!("Plugin path contains invalid UTF-8"))
}

async fn get_default_branch(plugin_dir: &Path) -> Result<String> {
    let output = run_git(
        ["symbolic-ref", "refs/remotes/origin/HEAD", "--short"],
        Some(plugin_dir),
        Duration::from_secs(10),
        "symbolic-ref",
    )
    .await?;

    let branch = output_branch_name(&output);
    if let Some(branch) = branch {
        return Ok(branch);
    }

    Ok("main".to_string())
}

fn output_branch_name(output: &std::process::Output) -> Option<String> {
    if !output.status.success() {
        return None;
    }

    let branch = String::from_utf8_lossy(&output.stdout);
    let branch = branch.trim().trim_start_matches("origin/");
    if !is_safe_branch_name(branch) {
        log::warn!("Invalid branch name from git: {:?}", branch);
        return None;
    }

    Some(branch.to_string())
}

async fn reset_to_origin_head(plugin_dir: &Path, branch: &str) -> Result<()> {
    let candidates = candidate_branches(branch);
    let mut last_error = String::new();

    for candidate in candidates {
        let output = reset_to_branch(plugin_dir, &candidate).await?;
        if output.status.success() {
            return Ok(());
        }
        last_error = String::from_utf8_lossy(&output.stderr).trim().to_string();
    }

    if last_error.is_empty() {
        anyhow::bail!("Git reset failed for origin/main and origin/master");
    }

    anyhow::bail!(
        "Git reset failed for origin/main and origin/master: {}",
        last_error
    )
}

fn candidate_branches(branch: &str) -> Vec<String> {
    let mut candidates = vec![branch.to_string()];
    if branch != "main" {
        candidates.push("main".to_string());
    }
    if branch != "master" {
        candidates.push("master".to_string());
    }
    candidates
}

async fn reset_to_branch(plugin_dir: &Path, branch: &str) -> Result<std::process::Output> {
    let reset_target = format!("origin/{}", branch);
    run_git(
        ["reset", "--hard", reset_target.as_str()],
        Some(plugin_dir),
        super::GIT_TIMEOUT,
        "reset",
    )
    .await
}

fn is_safe_branch_name(branch: &str) -> bool {
    !branch.is_empty()
        && branch.len() <= 256
        && branch.chars().all(|ch| {
            ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '/' || ch == '.'
        })
        && !branch.starts_with('-')
        && !branch.starts_with('.')
        && !branch.contains("..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_safe_branch_name_cases() {
        let valid = [
            "main",
            "master",
            "develop",
            "feature/foo",
            "release-1.0",
            "v1.0.0",
            "fix_bug",
            "a",
        ];
        for branch in valid {
            assert!(is_safe_branch_name(branch), "should be valid: {:?}", branch);
        }

        let invalid = [
            "",
            "-leading-dash",
            ".hidden",
            "has..double-dots",
            "has\nline",
            "has\ttab",
            "has space",
            &"x".repeat(300),
        ];
        for branch in invalid {
            assert!(
                !is_safe_branch_name(branch),
                "should be invalid: {:?}",
                branch
            );
        }
    }
}
