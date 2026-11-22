use anyhow::{Context, Result};
use qol_conventions::pr_try::{short_sha, ListedBuild};
use serde::Deserialize;

use super::download::gh;
use super::platform::Build;

const LIMIT: &str = "100";
const FIELDS: &str = "number,title,headRefOid,isCrossRepository,statusCheckRollup";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Listed {
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

pub(super) fn run(repo: &str, build: &Build, json: bool) -> Result<String> {
    let listing = gh(&[
        "pr", "list", "--repo", repo, "--state", "open", "--limit", LIMIT, "--json", FIELDS,
    ])?;
    let ready = ready(&listing, build.job)?;
    if json {
        return serde_json::to_string(&ready).context("writing the pull request list as JSON");
    }
    if ready.is_empty() {
        return Ok(format!(
            "no open pull request has a build for {}",
            build.label()
        ));
    }
    Ok(ready
        .iter()
        .map(|pull| format!("#{} {} {}", pull.number, pull.head, pull.title))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn ready(listing: &str, job: &str) -> Result<Vec<ListedBuild>> {
    let listed: Vec<Listed> =
        serde_json::from_str(listing).context("gh pr list printed something other than JSON")?;
    Ok(listed
        .into_iter()
        .filter(|pull| !pull.is_cross_repository && built(&pull.status_check_rollup, job))
        .map(|pull| ListedBuild {
            number: pull.number,
            head: short_sha(&pull.head_ref_oid).to_string(),
            title: pull.title,
        })
        .collect())
}

fn built(checks: &[Check], job: &str) -> bool {
    checks.iter().any(|check| {
        check.name.as_deref() == Some(job)
            && check
                .conclusion
                .as_deref()
                .is_some_and(|conclusion| conclusion.eq_ignore_ascii_case("success"))
    })
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
    fn only_branches_whose_build_for_this_machine_succeeded_are_ready() {
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
        assert_eq!(
            ready(&listing, JOB).unwrap(),
            vec![ListedBuild {
                number: 621,
                title: "Change 621".into(),
                head: "d3b28323f".into(),
            }]
        );
    }

    #[test]
    fn a_pull_request_without_checks_is_not_ready_and_bad_output_is_an_error() {
        let bare = r#"[{"number":5,"title":"x","headRefOid":"abc","isCrossRepository":false}]"#;
        assert!(ready(bare, JOB).unwrap().is_empty());
        assert!(ready("not json", JOB).is_err());
    }
}
