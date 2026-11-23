use anyhow::{bail, Context, Result};
use qol_conventions::pr_try::{short_sha, ListedBuild};
use std::path::Path;

use crate::platform::Build;

pub const WORKFLOW: &str = "live-test.yml";
pub const BUILD_JOB: &str = "Build for the live test";
pub const RUN_LIMIT: u32 = 20;
pub const LIST_LIMIT: u32 = 100;
const COMPLETED: &str = "completed";
const SUCCESS: &str = "success";
const FAILURE: &str = "failure";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequest {
    pub head_sha: String,
    pub head_branch: String,
    pub from_fork: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub id: u64,
    pub status: String,
    pub conclusion: String,
    pub head_branch: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenPull {
    pub number: u64,
    pub title: String,
    pub head_sha: String,
    pub from_fork: bool,
    pub built: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingArtifact(pub String);

impl std::fmt::Display for MissingArtifact {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for MissingArtifact {}

pub trait Source {
    fn pull_request(&self, number: u64) -> Result<PullRequest>;
    fn live_test_runs(&self, pull: &PullRequest) -> Result<Vec<Run>>;
    fn job_succeeded(&self, run: &Run, job: &str) -> Result<bool>;
    fn download(&self, run: &Run, artifact: &str, dir: &Path) -> Result<()>;
    fn open_pulls(&self, job: &str) -> Result<Vec<OpenPull>>;
}

pub(crate) fn trusted(pull: PullRequest, repo: &str, number: u64) -> Result<PullRequest> {
    if pull.from_fork {
        bail!(
            "pull request #{number} comes from a fork, so its build is code from outside {repo}; only branches of {repo} are tried"
        );
    }
    if pull.head_sha.is_empty() {
        bail!("pull request #{number} has no head commit");
    }
    Ok(pull)
}

pub(crate) fn live_test_run(source: &dyn Source, pull: &PullRequest, build: &Build) -> Result<Run> {
    let runs = source.live_test_runs(pull)?;
    latest_run(runs, &pull.head_sha, &pull.head_branch, |run| {
        source.job_succeeded(run, build.job)
    })
}

fn latest_run(
    runs: Vec<Run>,
    head: &str,
    branch: &str,
    host_built: impl Fn(&Run) -> Result<bool>,
) -> Result<Run> {
    let mut runs: Vec<Run> = runs
        .into_iter()
        .filter(|run| run.head_branch == branch)
        .collect();
    if runs.is_empty() {
        bail!("no live test run of {branch} exists for {head} yet");
    }
    let mut usable = None;
    for (index, run) in runs.iter().enumerate() {
        if usable_run(run, &host_built)? {
            usable = Some(index);
            break;
        }
    }
    if let Some(index) = usable {
        return Ok(runs.swap_remove(index));
    }
    let newest = runs.swap_remove(0);
    if newest.status != COMPLETED {
        bail!(
            "the live test run for {head} is still {}; try again when `{BUILD_JOB}` finishes: {}",
            newest.status,
            newest.url
        );
    }
    bail!(
        "the live test run for {head} ended {} before this machine's build finished: {}",
        newest.conclusion,
        newest.url
    )
}

fn usable_run(run: &Run, host_built: &impl Fn(&Run) -> Result<bool>) -> Result<bool> {
    if run.status != COMPLETED {
        return host_built(run);
    }
    match run.conclusion.as_str() {
        SUCCESS => Ok(true),
        FAILURE => host_built(run),
        _ => Ok(false),
    }
}

pub(crate) fn fetch(source: &dyn Source, run: &Run, build: &Build, dir: &Path) -> Result<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).with_context(|| format!("clearing {}", dir.display()))?;
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    source.download(run, build.artifact, dir).map_err(|error| {
        if error.downcast_ref::<MissingArtifact>().is_none() {
            return error;
        }
        error.context(format!(
            "the live test run uploaded no {}: nothing it changes runs on {}, or `{}` did not run; see {}",
            build.artifact,
            build.label(),
            build.job,
            run.url
        ))
    })
}

pub fn ready(source: &dyn Source, build: &Build) -> Result<Vec<ListedBuild>> {
    Ok(ready_among(source.open_pulls(build.job)?))
}

fn ready_among(pulls: Vec<OpenPull>) -> Vec<ListedBuild> {
    pulls
        .into_iter()
        .filter(|pull| !pull.from_fork && pull.built)
        .map(|pull| ListedBuild {
            number: pull.number,
            head: short_sha(&pull.head_sha).to_string(),
            title: pull.title,
        })
        .collect()
}

pub fn succeeded(conclusion: Option<&str>) -> bool {
    conclusion.is_some_and(|conclusion| conclusion.eq_ignore_ascii_case(SUCCESS))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BRANCH: &str = "feat/demo";

    fn run(id: u64, status: &str, conclusion: &str, branch: &str) -> Run {
        Run {
            id,
            status: status.into(),
            conclusion: conclusion.into(),
            head_branch: branch.into(),
            url: format!("https://example.invalid/{id}"),
        }
    }

    fn only(status: &str, conclusion: &str) -> Vec<Run> {
        vec![run(41, status, conclusion, BRANCH)]
    }

    fn never_built(_: &Run) -> Result<bool> {
        Ok(false)
    }

    fn pull(from_fork: bool) -> PullRequest {
        PullRequest {
            head_sha: "abc".into(),
            head_branch: BRANCH.into(),
            from_fork,
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    struct Downloads(fn() -> anyhow::Error);

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    impl Source for Downloads {
        fn pull_request(&self, _number: u64) -> Result<PullRequest> {
            Ok(pull(false))
        }

        fn live_test_runs(&self, _pull: &PullRequest) -> Result<Vec<Run>> {
            Ok(Vec::new())
        }

        fn job_succeeded(&self, _run: &Run, _job: &str) -> Result<bool> {
            Ok(true)
        }

        fn download(&self, _run: &Run, _artifact: &str, _dir: &Path) -> Result<()> {
            Err((self.0)())
        }

        fn open_pulls(&self, _job: &str) -> Result<Vec<OpenPull>> {
            Ok(Vec::new())
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn only_a_missing_artifact_is_explained_as_never_uploaded() {
        let root = tempfile::tempdir().unwrap();
        let build = crate::platform::host_build().unwrap();
        let failed = |source: &Downloads| {
            format!(
                "{:#}",
                fetch(
                    source,
                    &run(41, COMPLETED, SUCCESS, BRANCH),
                    build,
                    root.path()
                )
                .unwrap_err()
            )
        };
        let missing = failed(&Downloads(|| {
            MissingArtifact("run 41 has no build".into()).into()
        }));
        assert!(missing.contains("uploaded no"), "{missing}");
        assert!(missing.contains("run 41 has no build"), "{missing}");
        let expired = failed(&Downloads(|| {
            anyhow::anyhow!("the build of run 41 expired")
        }));
        assert_eq!(expired, "the build of run 41 expired");
    }

    #[test]
    fn a_branch_of_the_repository_is_trusted_and_a_fork_is_refused() {
        let trusted_pull = trusted(pull(false), "qol-tools/qol", 412).unwrap();
        assert_eq!(trusted_pull.head_sha, "abc");
        let error = trusted(pull(true), "qol-tools/qol", 412)
            .unwrap_err()
            .to_string();
        assert!(error.contains("from a fork"), "{error}");
        let headless = PullRequest {
            head_sha: String::new(),
            ..pull(false)
        };
        assert!(trusted(headless, "qol-tools/qol", 412).is_err());
    }

    #[test]
    fn a_finished_run_is_used() {
        let found = latest_run(only(COMPLETED, SUCCESS), "abc", BRANCH, never_built).unwrap();
        assert_eq!(found.id, 41);
    }

    #[test]
    fn a_run_of_another_branch_is_never_used() {
        let error = latest_run(only(COMPLETED, SUCCESS), "abc", "feat/other", never_built)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no live test run of feat/other"), "{error}");
    }

    #[test]
    fn a_newer_cancelled_run_does_not_hide_an_earlier_success() {
        let runs = vec![
            run(43, COMPLETED, "cancelled", BRANCH),
            run(42, COMPLETED, SUCCESS, "feat/other"),
            run(41, COMPLETED, SUCCESS, BRANCH),
        ];
        assert_eq!(latest_run(runs, "abc", BRANCH, never_built).unwrap().id, 41);
    }

    #[test]
    fn a_missing_running_or_failed_run_says_why() {
        let none = latest_run(Vec::new(), "abc", BRANCH, never_built)
            .unwrap_err()
            .to_string();
        assert!(none.contains("no live test run"), "{none}");
        let running = latest_run(only("in_progress", ""), "abc", BRANCH, never_built)
            .unwrap_err()
            .to_string();
        assert!(running.contains("still in_progress"), "{running}");
        assert!(running.contains(BUILD_JOB), "{running}");
        let failed = latest_run(only(COMPLETED, FAILURE), "abc", BRANCH, never_built)
            .unwrap_err()
            .to_string();
        assert!(failed.contains("ended failure"), "{failed}");
    }

    #[test]
    fn a_running_or_failed_run_is_used_once_this_machines_build_succeeded() {
        for status in ["in_progress", COMPLETED] {
            let runs = || {
                vec![
                    run(
                        43,
                        status,
                        if status == COMPLETED { FAILURE } else { "" },
                        BRANCH,
                    ),
                    run(41, COMPLETED, SUCCESS, BRANCH),
                ]
            };
            let built = |found: &Run| Ok(found.id == 43);
            assert_eq!(latest_run(runs(), "abc", BRANCH, built).unwrap().id, 43);
            assert_eq!(
                latest_run(runs(), "abc", BRANCH, never_built).unwrap().id,
                41
            );
        }
    }

    #[test]
    fn a_cancelled_run_is_never_asked_about_its_jobs() {
        let asked = |_: &Run| -> Result<bool> { panic!("asked about a cancelled run") };
        let error = latest_run(only(COMPLETED, "cancelled"), "abc", BRANCH, asked)
            .unwrap_err()
            .to_string();
        assert!(error.contains("ended cancelled"), "{error}");
    }

    #[test]
    fn only_branches_whose_build_for_this_machine_succeeded_are_ready() {
        let open = |number, from_fork, built| OpenPull {
            number,
            title: format!("Change {number}"),
            head_sha: "d3b28323f0123456".into(),
            from_fork,
            built,
        };
        assert_eq!(
            ready_among(vec![
                open(621, false, true),
                open(620, true, true),
                open(619, false, false)
            ]),
            vec![ListedBuild {
                number: 621,
                title: "Change 621".into(),
                head: "d3b28323f".into(),
            }]
        );
    }

    #[test]
    fn only_a_success_counts_as_built() {
        assert!(succeeded(Some("SUCCESS")));
        assert!(succeeded(Some("success")));
        assert!(!succeeded(Some("failure")));
        assert!(!succeeded(None));
    }
}
