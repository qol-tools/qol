use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::path::Path;
use std::process::Command;

const WORKFLOW: &str = "live-test.yml";
const ARTIFACT: &str = "live-test-build-linux-x86_64";
const BUILD_JOB: &str = "Build for the live test";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Run {
    database_id: u64,
    status: String,
    conclusion: String,
    url: String,
}

pub(super) fn head_sha(repo: &str, number: u64) -> Result<String> {
    let number = number.to_string();
    let sha = gh(&[
        "pr",
        "view",
        &number,
        "--repo",
        repo,
        "--json",
        "headRefOid",
        "--jq",
        ".headRefOid",
    ])?;
    let sha = sha.trim();
    if sha.is_empty() {
        bail!("pull request #{number} has no head commit");
    }
    Ok(sha.to_string())
}

pub(super) fn live_test_run(repo: &str, head: &str) -> Result<Run> {
    let listing = gh(&[
        "run",
        "list",
        "--repo",
        repo,
        "--workflow",
        WORKFLOW,
        "--commit",
        head,
        "--event",
        "pull_request",
        "--limit",
        "1",
        "--json",
        "databaseId,status,conclusion,url",
    ])?;
    latest_run(&listing, head)
}

fn latest_run(listing: &str, head: &str) -> Result<Run> {
    let runs: Vec<Run> =
        serde_json::from_str(listing).context("gh run list printed something other than JSON")?;
    let Some(run) = runs.into_iter().next() else {
        bail!("no live test run exists for {head} yet");
    };
    if run.status != "completed" {
        bail!(
            "the live test run for {head} is still {}; try again when `{BUILD_JOB}` finishes: {}",
            run.status,
            run.url
        );
    }
    if run.conclusion != "success" {
        bail!(
            "the live test run for {head} ended {}: {}",
            run.conclusion,
            run.url
        );
    }
    Ok(run)
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

    fn listing(status: &str, conclusion: &str) -> String {
        format!(
            r#"[{{"databaseId":41,"status":"{status}","conclusion":"{conclusion}","url":"https://example.invalid/41"}}]"#
        )
    }

    #[test]
    fn a_finished_run_is_used() {
        let run = latest_run(&listing("completed", "success"), "abc").unwrap();
        assert_eq!(run.database_id, 41);
    }

    #[test]
    fn a_missing_running_or_failed_run_says_why() {
        let none = latest_run("[]", "abc").unwrap_err().to_string();
        assert!(none.contains("no live test run"), "{none}");
        let running = latest_run(&listing("in_progress", ""), "abc")
            .unwrap_err()
            .to_string();
        assert!(running.contains("still in_progress"), "{running}");
        assert!(running.contains(BUILD_JOB), "{running}");
        let failed = latest_run(&listing("completed", "failure"), "abc")
            .unwrap_err()
            .to_string();
        assert!(failed.contains("ended failure"), "{failed}");
    }
}
