use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::Path;
use std::process::Command;

const WORKFLOW: &str = "live-test.yml";
const ARTIFACT: &str = "live-test-build-linux-x86_64";
const BUILD_JOB: &str = "Build for the live test";
const RUN_LIMIT: &str = "20";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Run {
    database_id: u64,
    status: String,
    conclusion: String,
    head_branch: String,
    url: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PullRequest {
    pub(super) head_ref_oid: String,
    pub(super) head_ref_name: String,
    is_cross_repository: bool,
}

pub(super) fn pull_request(repo: &str, number: u64) -> Result<PullRequest> {
    let number = number.to_string();
    let view = gh(&[
        "pr",
        "view",
        &number,
        "--repo",
        repo,
        "--json",
        "headRefOid,headRefName,isCrossRepository",
    ])?;
    trusted(&view, repo, &number)
}

fn trusted(view: &str, repo: &str, number: &str) -> Result<PullRequest> {
    let pull: PullRequest =
        serde_json::from_str(view).context("gh pr view printed something other than JSON")?;
    if pull.is_cross_repository {
        bail!(
            "pull request #{number} comes from a fork, so its build is code from outside {repo}; qol pr try runs only branches of {repo}"
        );
    }
    if pull.head_ref_oid.is_empty() {
        bail!("pull request #{number} has no head commit");
    }
    Ok(pull)
}

pub(super) fn live_test_run(repo: &str, pull: &PullRequest) -> Result<Run> {
    let head = pull.head_ref_oid.as_str();
    let listing = gh(&[
        "run",
        "list",
        "--repo",
        repo,
        "--workflow",
        WORKFLOW,
        "--commit",
        head,
        "--branch",
        &pull.head_ref_name,
        "--event",
        "pull_request",
        "--limit",
        RUN_LIMIT,
        "--json",
        "databaseId,status,conclusion,headBranch,url",
    ])?;
    latest_run(&listing, head, &pull.head_ref_name)
}

fn latest_run(listing: &str, head: &str, branch: &str) -> Result<Run> {
    let runs: Vec<Run> =
        serde_json::from_str(listing).context("gh run list printed something other than JSON")?;
    let mut runs = runs.into_iter().filter(|run| run.head_branch == branch);
    let Some(newest) = runs.next() else {
        bail!("no live test run of {branch} exists for {head} yet");
    };
    if newest.status == "completed" && newest.conclusion == "success" {
        return Ok(newest);
    }
    if let Some(earlier) = runs.find(|run| run.status == "completed" && run.conclusion == "success")
    {
        return Ok(earlier);
    }
    if newest.status != "completed" {
        bail!(
            "the live test run for {head} is still {}; try again when `{BUILD_JOB}` finishes: {}",
            newest.status,
            newest.url
        );
    }
    bail!(
        "the live test run for {head} ended {}: {}",
        newest.conclusion,
        newest.url
    )
}

pub(super) fn fetch(repo: &str, run: &Run, dir: &Path) -> Result<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).with_context(|| format!("clearing {}", dir.display()))?;
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let id = run.database_id.to_string();
    let dir_arg = dir.to_string_lossy();
    gh(&[
        "run", "download", &id, "--repo", repo, "--name", ARTIFACT, "--dir", &dir_arg,
    ])
    .with_context(|| {
        format!(
            "the live test run uploaded no {ARTIFACT}: nothing it changes runs on Linux, or `{BUILD_JOB}` did not run; see {}",
            run.url
        )
    })?;
    Ok(())
}

fn gh(args: &[&str]) -> Result<String> {
    let output = Command::new("gh")
        .args(args)
        .output()
        .context("could not run gh; install the GitHub CLI and run `gh auth login`")?;
    if !output.status.success() {
        bail!(
            "gh {} failed: {}",
            args.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("gh printed non-UTF-8 output")
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRANCH: &str = "feat/demo";

    fn entry(id: u64, status: &str, conclusion: &str, branch: &str) -> String {
        format!(
            r#"{{"databaseId":{id},"status":"{status}","conclusion":"{conclusion}","headBranch":"{branch}","url":"https://example.invalid/{id}"}}"#
        )
    }

    fn listing(status: &str, conclusion: &str) -> String {
        format!("[{}]", entry(41, status, conclusion, BRANCH))
    }

    fn view(cross: bool) -> String {
        format!(r#"{{"headRefOid":"abc","headRefName":"{BRANCH}","isCrossRepository":{cross}}}"#)
    }

    #[test]
    fn a_branch_of_the_repository_is_trusted() {
        let pull = trusted(&view(false), "qol-tools/qol", "412").unwrap();
        assert_eq!(pull.head_ref_oid, "abc");
        assert_eq!(pull.head_ref_name, BRANCH);
    }

    #[test]
    fn a_fork_is_refused() {
        let error = trusted(&view(true), "qol-tools/qol", "412")
            .unwrap_err()
            .to_string();
        assert!(error.contains("from a fork"), "{error}");
    }

    #[test]
    fn a_finished_run_is_used() {
        let run = latest_run(&listing("completed", "success"), "abc", BRANCH).unwrap();
        assert_eq!(run.database_id, 41);
    }

    #[test]
    fn a_run_of_another_branch_is_never_used() {
        let error = latest_run(&listing("completed", "success"), "abc", "feat/other")
            .unwrap_err()
            .to_string();
        assert!(error.contains("no live test run of feat/other"), "{error}");
    }

    #[test]
    fn a_newer_cancelled_run_does_not_hide_an_earlier_success() {
        let runs = format!(
            "[{},{},{}]",
            entry(43, "completed", "cancelled", BRANCH),
            entry(42, "completed", "success", "feat/other"),
            entry(41, "completed", "success", BRANCH)
        );
        assert_eq!(latest_run(&runs, "abc", BRANCH).unwrap().database_id, 41);
    }

    #[test]
    fn a_missing_running_or_failed_run_says_why() {
        let none = latest_run("[]", "abc", BRANCH).unwrap_err().to_string();
        assert!(none.contains("no live test run"), "{none}");
        let running = latest_run(&listing("in_progress", ""), "abc", BRANCH)
            .unwrap_err()
            .to_string();
        assert!(running.contains("still in_progress"), "{running}");
        assert!(running.contains(BUILD_JOB), "{running}");
        let failed = latest_run(&listing("completed", "failure"), "abc", BRANCH)
            .unwrap_err()
            .to_string();
        assert!(failed.contains("ended failure"), "{failed}");
    }
}
