use anyhow::{bail, Context, Result};
use qol_pr_try::{OpenPull, PullRequest, Run, Source, LIST_LIMIT, RUN_LIMIT, WORKFLOW};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use std::io::Write;
use std::path::Path;

use super::{build_github_request, send_checked};
use crate::features::connection_health::observe_body;

const API: &str = "https://api.github.com/repos";
const JOBS_PER_PAGE: u32 = 100;
const PULL_REQUEST_EVENT: &str = "pull_request";

pub(crate) struct Actions {
    repo: &'static str,
    token: String,
    client: reqwest::Client,
    runtime: tokio::runtime::Handle,
}

#[derive(Debug, Deserialize)]
struct Repository {
    full_name: String,
}

#[derive(Debug, Deserialize)]
struct Head {
    sha: String,
    #[serde(rename = "ref", default)]
    branch: String,
    #[serde(default)]
    repo: Option<Repository>,
}

#[derive(Debug, Deserialize)]
struct Pull {
    #[serde(default)]
    number: u64,
    #[serde(default)]
    title: String,
    head: Head,
}

#[derive(Debug, Deserialize)]
struct WorkflowRuns {
    workflow_runs: Vec<WorkflowRun>,
}

#[derive(Debug, Deserialize)]
struct WorkflowRun {
    id: u64,
    status: Option<String>,
    conclusion: Option<String>,
    head_branch: Option<String>,
    html_url: String,
}

#[derive(Debug, Deserialize)]
struct Jobs {
    jobs: Vec<Job>,
}

#[derive(Debug, Deserialize)]
struct Job {
    name: String,
    conclusion: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Artifacts {
    artifacts: Vec<Artifact>,
}

#[derive(Debug, Deserialize)]
struct Artifact {
    id: u64,
    name: String,
    #[serde(default)]
    expired: bool,
}

#[derive(Debug, Deserialize)]
struct CheckRuns {
    check_runs: Vec<Job>,
}

impl Actions {
    pub(crate) fn new(repo: &'static str, token: String, runtime: tokio::runtime::Handle) -> Self {
        Self {
            repo,
            token,
            client: crate::features::plugin_store::blob_client(),
            runtime,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{API}/{}/{path}", self.repo)
    }

    fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, String)]) -> Result<T> {
        let request =
            build_github_request(&self.client, &self.url(path), Some(&self.token)).query(query);
        let body = self.runtime.block_on(async {
            let response = send_checked(request).await?;
            anyhow::Ok(observe_body(response).bytes().await?)
        })?;
        serde_json::from_slice(&body)
            .with_context(|| format!("GitHub answered {path} with something unreadable"))
    }

    fn save(&self, path: &str, file: &mut std::fs::File) -> Result<()> {
        let request = build_github_request(&self.client, &self.url(path), Some(&self.token));
        self.runtime.block_on(async {
            let response = send_checked(request).await?;
            let mut body = observe_body(response);
            while let Some(chunk) = body.chunk().await? {
                file.write_all(chunk.as_ref())?;
            }
            file.sync_all()?;
            anyhow::Ok(())
        })
    }
}

impl Source for Actions {
    fn pull_request(&self, number: u64) -> Result<PullRequest> {
        let pull: Pull = self.get(&format!("pulls/{number}"), &[])?;
        Ok(pull_request(pull, self.repo))
    }

    fn live_test_runs(&self, pull: &PullRequest) -> Result<Vec<Run>> {
        let runs: WorkflowRuns = self.get(
            &format!("actions/workflows/{WORKFLOW}/runs"),
            &[
                ("head_sha", pull.head_sha.clone()),
                ("branch", pull.head_branch.clone()),
                ("event", PULL_REQUEST_EVENT.to_string()),
                ("per_page", RUN_LIMIT.to_string()),
            ],
        )?;
        Ok(runs.workflow_runs.into_iter().map(run).collect())
    }

    fn job_succeeded(&self, run: &Run, job: &str) -> Result<bool> {
        let jobs: Jobs = self.get(
            &format!("actions/runs/{}/jobs", run.id),
            &[("per_page", JOBS_PER_PAGE.to_string())],
        )?;
        Ok(any_succeeded(&jobs.jobs, job))
    }

    fn download(&self, run: &Run, artifact: &str, dir: &Path) -> Result<()> {
        let listed: Artifacts = self.get(
            &format!("actions/runs/{}/artifacts", run.id),
            &[("name", artifact.to_string())],
        )?;
        let found = usable_artifact(listed, artifact, run.id)?;
        let staging = dir.parent().unwrap_or(dir);
        let mut archive = tempfile::NamedTempFile::new_in(staging)
            .with_context(|| format!("staging the download in {}", staging.display()))?;
        self.save(
            &format!("actions/artifacts/{}/zip", found.id),
            archive.as_file_mut(),
        )
        .with_context(|| format!("downloading {artifact}"))?;
        qol_pr_try::unpack_zip(archive.path(), dir)
    }

    fn open_pulls(&self, job: &str) -> Result<Vec<OpenPull>> {
        let pulls: Vec<Pull> = self.get(
            "pulls",
            &[
                ("state", "open".to_string()),
                ("per_page", LIST_LIMIT.to_string()),
            ],
        )?;
        let mut open = Vec::new();
        for pull in pulls {
            let number = pull.number;
            let title = pull.title.clone();
            let head = pull_request(pull, self.repo);
            let built = !head.from_fork && self.built(&head.head_sha, job)?;
            open.push(OpenPull {
                number,
                title,
                head_sha: head.head_sha,
                from_fork: head.from_fork,
                built,
            });
        }
        Ok(open)
    }
}

impl Actions {
    fn built(&self, sha: &str, job: &str) -> Result<bool> {
        let checks: CheckRuns = self.get(
            &format!("commits/{sha}/check-runs"),
            &[
                ("check_name", job.to_string()),
                ("filter", "latest".to_string()),
            ],
        )?;
        Ok(any_succeeded(&checks.check_runs, job))
    }
}

fn pull_request(pull: Pull, repo: &str) -> PullRequest {
    let from_fork = pull
        .head
        .repo
        .as_ref()
        .is_none_or(|head| !head.full_name.eq_ignore_ascii_case(repo));
    PullRequest {
        head_sha: pull.head.sha,
        head_branch: pull.head.branch,
        from_fork,
    }
}

fn run(run: WorkflowRun) -> Run {
    Run {
        id: run.id,
        status: run.status.unwrap_or_default(),
        conclusion: run.conclusion.unwrap_or_default(),
        head_branch: run.head_branch.unwrap_or_default(),
        url: run.html_url,
    }
}

fn any_succeeded(jobs: &[Job], name: &str) -> bool {
    jobs.iter()
        .any(|job| job.name == name && qol_pr_try::succeeded(job.conclusion.as_deref()))
}

fn usable_artifact(listed: Artifacts, name: &str, run: u64) -> Result<Artifact> {
    let found = listed
        .artifacts
        .into_iter()
        .find(|artifact| artifact.name == name)
        .with_context(|| format!("run {run} has no {name}"))?;
    if found.expired {
        bail!(
            "the {name} build of run {run} expired; a new push to the pull request builds it again"
        );
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = "qol-tools/qol";

    fn pull(head_repo: Option<&str>) -> Pull {
        let repo = head_repo.map_or("null".to_string(), |name| {
            format!(r#"{{"full_name":"{name}"}}"#)
        });
        serde_json::from_str(&format!(
            r#"{{"number":621,"title":"Change","head":{{"sha":"abc","ref":"feat/demo","repo":{repo}}}}}"#
        ))
        .unwrap()
    }

    #[test]
    fn only_a_branch_of_the_repository_is_not_a_fork() {
        let own = pull_request(pull(Some(REPO)), REPO);
        assert_eq!(own.head_sha, "abc");
        assert_eq!(own.head_branch, "feat/demo");
        assert!(!own.from_fork);
        assert!(pull_request(pull(Some("someone/qol")), REPO).from_fork);
        assert!(pull_request(pull(None), REPO).from_fork);
    }

    #[test]
    fn a_run_reads_with_its_missing_fields_empty() {
        let runs: WorkflowRuns = serde_json::from_str(
            r#"{"total_count":1,"workflow_runs":[{"id":43,"status":"in_progress","conclusion":null,"head_branch":"feat/demo","html_url":"https://github.com/qol-tools/qol/actions/runs/43"}]}"#,
        )
        .unwrap();
        let read: Vec<Run> = runs.workflow_runs.into_iter().map(run).collect();
        assert_eq!(read[0].id, 43);
        assert_eq!(read[0].status, "in_progress");
        assert_eq!(read[0].conclusion, "");
        assert!(read[0].url.ends_with("/43"));
    }

    #[test]
    fn only_this_machines_successful_job_counts() {
        let jobs: Jobs = serde_json::from_str(
            r#"{"jobs":[{"name":"Build for the live test (linux)","conclusion":"failure"},{"name":"Build for the live test (macos)","conclusion":"success"},{"name":"Build for the live test (linux)","conclusion":null}]}"#,
        )
        .unwrap();
        assert!(any_succeeded(&jobs.jobs, "Build for the live test (macos)"));
        assert!(!any_succeeded(
            &jobs.jobs,
            "Build for the live test (linux)"
        ));
    }

    #[test]
    fn a_missing_or_expired_artifact_says_why() {
        let listed = |expired: bool| -> Artifacts {
            serde_json::from_str(&format!(
                r#"{{"artifacts":[{{"id":9,"name":"live-test-build-linux-x86_64","expired":{expired}}}]}}"#
            ))
            .unwrap()
        };
        let found = usable_artifact(listed(false), "live-test-build-linux-x86_64", 43).unwrap();
        assert_eq!(found.id, 9);
        let expired = usable_artifact(listed(true), "live-test-build-linux-x86_64", 43)
            .unwrap_err()
            .to_string();
        assert!(expired.contains("expired"), "{expired}");
        let missing = usable_artifact(listed(false), "live-test-build-macos-aarch64", 43)
            .unwrap_err()
            .to_string();
        assert!(
            missing.contains("has no live-test-build-macos-aarch64"),
            "{missing}"
        );
    }
}
