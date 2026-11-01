use super::{http_client, GITHUB_REPO};
use crate::features::connection_health::SendObserved;
use serde::de::DeserializeOwned;
use serde::Deserialize;

pub(super) fn is_running() -> bool {
    commits_match(installed_commit().as_deref(), running_commit())
}

pub(super) async fn precedes_release(release: &str) -> bool {
    let Some(commit) = running_commit() else {
        return false;
    };
    match placement(compare(release, commit).await.as_deref()) {
        Placement::Behind => true,
        Placement::Diverged => merged_into(release, commit).await,
        Placement::Kept => false,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Placement {
    Behind,
    Diverged,
    Kept,
}

fn placement(status: Option<&str>) -> Placement {
    match status {
        Some("behind") => Placement::Behind,
        Some("diverged") => Placement::Diverged,
        _ => Placement::Kept,
    }
}

async fn merged_into(release: &str, commit: &str) -> bool {
    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/commits/{commit}/pulls");
    let Some(pulls) = get_json::<Vec<PullRequest>>(&url).await else {
        return false;
    };
    for merge in merge_commits(&pulls) {
        if contains(compare(release, merge).await.as_deref()) {
            return true;
        }
    }
    false
}

#[derive(Debug, Deserialize)]
struct PullRequest {
    merged_at: Option<String>,
    merge_commit_sha: Option<String>,
}

fn merge_commits(pulls: &[PullRequest]) -> impl Iterator<Item = &str> {
    pulls
        .iter()
        .filter(|pull| pull.merged_at.is_some())
        .filter_map(|pull| pull.merge_commit_sha.as_deref())
}

fn contains(status: Option<&str>) -> bool {
    matches!(status, Some("behind" | "identical"))
}

#[derive(Debug, Deserialize)]
struct Comparison {
    status: Option<String>,
}

async fn compare(release: &str, commit: &str) -> Option<String> {
    let url = format!("https://api.github.com/repos/{GITHUB_REPO}/compare/{release}...{commit}");
    get_json::<Comparison>(&url).await?.status
}

async fn get_json<T: DeserializeOwned>(url: &str) -> Option<T> {
    let token = crate::credentials::github_bearer_token();
    let request = crate::features::plugin_store::github::build_github_request(
        http_client(),
        url,
        token.as_deref(),
    );
    let response = request.send_observed().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    response.json().await.ok()
}

fn commits_match(installed: Option<&str>, running: Option<&str>) -> bool {
    matches!((installed, running), (Some(installed), Some(running)) if installed == running)
}

fn installed_commit() -> Option<String> {
    let path = qol_config::data_dir()?.join(qol_conventions::HOST_WORKSPACE_BUILD_FILE);
    let commit = std::fs::read_to_string(path).ok()?;
    let commit = commit.trim();
    (!commit.is_empty()).then(|| commit.to_string())
}

fn running_commit() -> Option<&'static str> {
    match &qol_conventions::artifact::current()?.source {
        qol_conventions::artifact::SourceIdentity::Git { commit, .. } => Some(commit),
        qol_conventions::artifact::SourceIdentity::Unspecified => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_running_workspace_build_is_detected() {
        let cases = [
            (Some("abc123"), Some("abc123"), true),
            (Some("abc123"), Some("def456"), false),
            (None, Some("abc123"), false),
            (Some("abc123"), None, false),
            (None, None, false),
        ];
        for (installed, running, expected) in cases {
            assert_eq!(
                commits_match(installed, running),
                expected,
                "installed={installed:?} running={running:?}"
            );
        }
    }

    #[test]
    fn a_diverged_build_asks_whether_its_pull_request_was_merged() {
        let cases = [
            (Some("behind"), Placement::Behind),
            (Some("diverged"), Placement::Diverged),
            (Some("ahead"), Placement::Kept),
            (Some("identical"), Placement::Kept),
            (None, Placement::Kept),
        ];
        for (status, expected) in cases {
            assert_eq!(placement(status), expected, "status={status:?}");
        }
    }

    #[test]
    fn a_release_holds_a_merge_it_is_level_with_or_past() {
        let cases = [
            (Some("behind"), true),
            (Some("identical"), true),
            (Some("ahead"), false),
            (Some("diverged"), false),
            (None, false),
        ];
        for (status, expected) in cases {
            assert_eq!(contains(status), expected, "status={status:?}");
        }
    }

    #[test]
    fn only_merged_pull_requests_name_a_merge_commit() {
        let pulls: Vec<PullRequest> = serde_json::from_str(
            r#"[
                {"number":375,"merged_at":"2026-10-30T21:47:51Z","merge_commit_sha":"496735928"},
                {"number":376,"merged_at":null,"merge_commit_sha":"0123abcd"},
                {"number":377,"merged_at":"2026-10-30T22:00:00Z","merge_commit_sha":null}
            ]"#,
        )
        .unwrap();
        assert_eq!(merge_commits(&pulls).collect::<Vec<_>>(), ["496735928"]);
    }
}
