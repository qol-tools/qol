use anyhow::{bail, Context, Result};
use qol_pr_try::{OpenPull, PullRequest, Run, Source, LIST_LIMIT, RUN_LIMIT, WORKFLOW};
use serde::Deserialize;
use std::path::Path;
use std::process::Command;

const PULL_FIELDS: &str = "headRefOid,headRefName,isCrossRepository";
const RUN_FIELDS: &str = "databaseId,status,conclusion,headBranch,url";
const LIST_FIELDS: &str = "number,title,headRefOid,isCrossRepository,statusCheckRollup";

pub(super) struct Gh {
    repo: &'static str,
}

impl Gh {
    pub(super) fn new(repo: &'static str) -> Self {
        Self { repo }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ViewedPull {
    head_ref_oid: String,
    head_ref_name: String,
    is_cross_repository: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedRun {
    database_id: u64,
    status: String,
    conclusion: String,
    head_branch: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct Jobs {
    jobs: Vec<Job>,
}

#[derive(Debug, Deserialize)]
struct Job {
    name: String,
    conclusion: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedPull {
    number: u64,
    title: String,
    head_ref_oid: String,
    is_cross_repository: bool,
    #[serde(default)]
    status_check_rollup: Vec<Check>,
}

#[derive(Debug, Deserialize)]
struct Check {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
}

impl Source for Gh {
    fn pull_request(&self, number: u64) -> Result<PullRequest> {
        let number = number.to_string();
        let view = gh(&[
            "pr",
            "view",
            &number,
            "--repo",
            self.repo,
            "--json",
            PULL_FIELDS,
        ])?;
        viewed_pull(&view)
    }

    fn live_test_runs(&self, pull: &PullRequest) -> Result<Vec<Run>> {
        let limit = RUN_LIMIT.to_string();
        let listing = gh(&[
            "run",
            "list",
            "--repo",
            self.repo,
            "--workflow",
            WORKFLOW,
            "--commit",
            &pull.head_sha,
            "--branch",
            &pull.head_branch,
            "--event",
            "pull_request",
            "--limit",
            &limit,
            "--json",
            RUN_FIELDS,
        ])?;
        listed_runs(&listing)
    }

    fn job_succeeded(&self, run: &Run, job: &str) -> Result<bool> {
        let id = run.id.to_string();
        let view = gh(&["run", "view", &id, "--repo", self.repo, "--json", "jobs"])?;
        job_succeeded(&view, job)
    }

    fn download(&self, run: &Run, artifact: &str, dir: &Path) -> Result<()> {
        let id = run.id.to_string();
        let dir_arg = dir.to_string_lossy();
        gh(&[
            "run", "download", &id, "--repo", self.repo, "--name", artifact, "--dir", &dir_arg,
        ])
        .map(drop)
    }

    fn open_pulls(&self, job: &str) -> Result<Vec<OpenPull>> {
        let limit = LIST_LIMIT.to_string();
        let listing = gh(&[
            "pr",
            "list",
            "--repo",
            self.repo,
            "--state",
            "open",
            "--limit",
            &limit,
            "--json",
            LIST_FIELDS,
        ])?;
        open_pulls(&listing, job)
    }
}

fn viewed_pull(view: &str) -> Result<PullRequest> {
    let pull: ViewedPull =
        serde_json::from_str(view).context("gh pr view printed something other than JSON")?;
    Ok(PullRequest {
        head_sha: pull.head_ref_oid,
        head_branch: pull.head_ref_name,
        from_fork: pull.is_cross_repository,
    })
}

fn listed_runs(listing: &str) -> Result<Vec<Run>> {
    let runs: Vec<ListedRun> =
        serde_json::from_str(listing).context("gh run list printed something other than JSON")?;
    Ok(runs
        .into_iter()
        .map(|run| Run {
            id: run.database_id,
            status: run.status,
            conclusion: run.conclusion,
            head_branch: run.head_branch,
            url: run.url,
        })
        .collect())
}

fn job_succeeded(view: &str, job: &str) -> Result<bool> {
    let view: Jobs =
        serde_json::from_str(view).context("gh run view printed something other than JSON")?;
    Ok(view
        .jobs
        .iter()
        .any(|entry| entry.name == job && qol_pr_try::succeeded(Some(&entry.conclusion))))
}

fn open_pulls(listing: &str, job: &str) -> Result<Vec<OpenPull>> {
    let listed: Vec<ListedPull> =
        serde_json::from_str(listing).context("gh pr list printed something other than JSON")?;
    Ok(listed
        .into_iter()
        .map(|pull| OpenPull {
            built: pull.status_check_rollup.iter().any(|check| {
                check.name.as_deref() == Some(job)
                    && qol_pr_try::succeeded(check.conclusion.as_deref())
            }),
            number: pull.number,
            title: pull.title,
            head_sha: pull.head_ref_oid,
            from_fork: pull.is_cross_repository,
        })
        .collect())
}

pub(super) fn gh(args: &[&str]) -> Result<String> {
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

    const JOB: &str = "Build for the live test (linux)";

    fn pull(number: u64, cross: bool, checks: &str) -> String {
        format!(
            r#"{{"number":{number},"title":"Change {number}","headRefOid":"d3b28323f0123456","isCrossRepository":{cross},"statusCheckRollup":[{checks}]}}"#
        )
    }

    fn check(name: &str, conclusion: &str) -> String {
        format!(
            r#"{{"__typename":"CheckRun","name":"{name}","status":"COMPLETED","conclusion":"{conclusion}"}}"#
        )
    }

    #[test]
    fn a_viewed_pull_request_names_its_head_and_whether_it_is_a_fork() {
        let pull = viewed_pull(
            r#"{"headRefOid":"abc","headRefName":"feat/demo","isCrossRepository":true}"#,
        )
        .unwrap();
        assert_eq!(pull.head_sha, "abc");
        assert_eq!(pull.head_branch, "feat/demo");
        assert!(pull.from_fork);
        assert!(viewed_pull("not json").is_err());
    }

    #[test]
    fn listed_runs_keep_their_order_and_fields() {
        let runs = listed_runs(
            r#"[{"databaseId":43,"status":"in_progress","conclusion":"","headBranch":"feat/demo","url":"https://example.invalid/43"},
                {"databaseId":41,"status":"completed","conclusion":"success","headBranch":"feat/demo","url":"https://example.invalid/41"}]"#,
        )
        .unwrap();
        assert_eq!(runs.iter().map(|run| run.id).collect::<Vec<_>>(), [43, 41]);
        assert_eq!(runs[1].conclusion, "success");
        assert!(listed_runs("not json").is_err());
    }

    #[test]
    fn only_this_machines_successful_build_job_counts() {
        let view = r#"{"jobs":[
            {"name":"Build for the live test (linux)","conclusion":"failure"},
            {"name":"Build for the live test (macos)","conclusion":"success"}
        ]}"#;
        assert!(job_succeeded(view, "Build for the live test (macos)").unwrap());
        assert!(!job_succeeded(view, "Build for the live test (linux)").unwrap());
        assert!(!job_succeeded(r#"{"jobs":[]}"#, "Build for the live test (macos)").unwrap());
    }

    #[test]
    fn an_open_pull_request_is_built_only_by_this_machines_successful_job() {
        let running = r#"{"__typename":"CheckRun","name":"Build for the live test (linux)","status":"IN_PROGRESS","conclusion":null}"#;
        let status = r#"{"__typename":"StatusContext","context":"merge gate","state":"SUCCESS"}"#;
        let listing = format!(
            "[{},{},{},{},{},{}]",
            pull(621, false, &check(JOB, "SUCCESS")),
            pull(620, true, &check(JOB, "SUCCESS")),
            pull(
                619,
                false,
                &check("Build for the live test (macos)", "SUCCESS")
            ),
            pull(618, false, &check(JOB, "FAILURE")),
            pull(617, false, running),
            pull(616, false, status),
        );
        let built: Vec<(u64, bool, bool)> = open_pulls(&listing, JOB)
            .unwrap()
            .into_iter()
            .map(|pull| (pull.number, pull.from_fork, pull.built))
            .collect();
        assert_eq!(
            built,
            [
                (621, false, true),
                (620, true, true),
                (619, false, false),
                (618, false, false),
                (617, false, false),
                (616, false, false)
            ]
        );
    }

    #[test]
    fn a_pull_request_without_checks_is_not_built_and_bad_output_is_an_error() {
        let bare = r#"[{"number":5,"title":"x","headRefOid":"abc","isCrossRepository":false}]"#;
        assert!(!open_pulls(bare, JOB).unwrap()[0].built);
        assert!(open_pulls("not json", JOB).is_err());
    }
}
