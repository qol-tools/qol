use anyhow::{anyhow, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};

use super::{ProviderError, RemoteDocument};
use crate::features::profile::sync::{GitHubSyncConnection, SyncBranchList};

pub(super) fn validate_connection(connection: &GitHubSyncConnection) -> Result<()> {
    parse_github_repo(&connection.repo_url)?;
    if !is_safe_branch(&connection.branch) {
        anyhow::bail!("Invalid branch");
    }
    if !super::is_safe_remote_path(&connection.path) {
        anyhow::bail!("Invalid remote path");
    }
    Ok(())
}

pub(super) async fn validate_github_token(token: &str) -> Result<()> {
    crate::features::plugin_store::github::validate_token(token)
        .await
        .map_err(|error| anyhow!(error.to_string()))
}

pub(super) fn resolve_github_token(github_token: Option<&str>) -> Result<String> {
    if let Some(token) = github_token {
        return Ok(token.to_string());
    }
    crate::credentials::github_bearer_token()
        .ok_or_else(|| anyhow!("GitHub credential is not configured"))
}

pub(super) fn normalize_repo_url(repo_url: &str) -> Result<String> {
    let trimmed = repo_url.trim();
    if trimmed.is_empty() {
        anyhow::bail!("Repo URL cannot be empty");
    }
    parse_github_repo(trimmed)?;
    Ok(trimmed.to_string())
}

pub(super) fn normalize_requested_branch(branch: &str) -> Result<Option<String>> {
    let trimmed = branch.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if !is_safe_branch(trimmed) {
        anyhow::bail!("Invalid branch");
    }
    Ok(Some(trimmed.to_string()))
}

pub(super) fn parse_github_repo(repo_url: &str) -> Result<(String, String)> {
    if let Some(rest) = repo_url.strip_prefix("https://github.com/") {
        return parse_owner_repo(rest);
    }
    if let Some(rest) = repo_url.strip_prefix("http://github.com/") {
        return parse_owner_repo(rest);
    }
    if let Some(rest) = repo_url.strip_prefix("git@github.com:") {
        return parse_owner_repo(rest);
    }
    if let Some(rest) = repo_url.strip_prefix("ssh://git@github.com/") {
        return parse_owner_repo(rest);
    }
    if let Some(rest) = repo_url.strip_prefix("ssh://git@ssh.github.com:443/") {
        return parse_owner_repo(rest);
    }
    anyhow::bail!("Repo URL must point to a GitHub repository")
}

pub(super) async fn fetch_github_default_branch(
    client: &reqwest::Client,
    repo_url: &str,
    token: &str,
) -> std::result::Result<String, ProviderError> {
    let (owner, repo) =
        parse_github_repo(repo_url).map_err(|error| ProviderError::Invalid(error.to_string()))?;
    let url = format!("https://api.github.com/repos/{owner}/{repo}");

    #[derive(Deserialize)]
    struct ResponseBody {
        default_branch: String,
    }

    let response = client
        .get(url)
        .header("User-Agent", "qol-tray")
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|error| ProviderError::Upstream(error.to_string()))?;

    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(ProviderError::Invalid(
            "GitHub repository was not found".to_string(),
        ));
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        let body = response.text().await.unwrap_or_default();
        return Err(ProviderError::Auth(format!(
            "GitHub authentication failed: {} {}",
            status, body
        )));
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(ProviderError::Upstream(format!(
            "GitHub returned {}: {}",
            status, body
        )));
    }

    let body: ResponseBody = response
        .json()
        .await
        .map_err(|error| ProviderError::Upstream(error.to_string()))?;
    if !is_safe_branch(&body.default_branch) {
        return Err(ProviderError::Invalid(
            "GitHub repository returned an invalid default branch".to_string(),
        ));
    }
    Ok(body.default_branch)
}

pub(super) async fn fetch_github_branches(
    client: &reqwest::Client,
    repo_url: &str,
    token: &str,
) -> std::result::Result<SyncBranchList, ProviderError> {
    let default_branch = fetch_github_default_branch(client, repo_url, token).await?;
    let (owner, repo) =
        parse_github_repo(repo_url).map_err(|error| ProviderError::Invalid(error.to_string()))?;
    let url = format!("https://api.github.com/repos/{owner}/{repo}/branches");

    #[derive(Deserialize)]
    struct ResponseBody {
        name: String,
    }

    let response = client
        .get(url)
        .query(&[("per_page", "100")])
        .header("User-Agent", "qol-tray")
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|error| ProviderError::Upstream(error.to_string()))?;

    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(ProviderError::Invalid(
            "GitHub repository was not found".to_string(),
        ));
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        let body = response.text().await.unwrap_or_default();
        return Err(ProviderError::Auth(format!(
            "GitHub authentication failed: {} {}",
            status, body
        )));
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(ProviderError::Upstream(format!(
            "GitHub returned {}: {}",
            status, body
        )));
    }

    let body: Vec<ResponseBody> = response
        .json()
        .await
        .map_err(|error| ProviderError::Upstream(error.to_string()))?;
    let mut branches = body
        .into_iter()
        .filter_map(|branch| {
            if !is_safe_branch(&branch.name) {
                return None;
            }
            Some(branch.name)
        })
        .collect::<Vec<_>>();
    if !branches.iter().any(|branch| branch == &default_branch) {
        branches.push(default_branch.clone());
    }
    branches.sort();
    if let Some(index) = branches.iter().position(|branch| branch == &default_branch) {
        let branch = branches.remove(index);
        branches.insert(0, branch);
    }

    Ok(SyncBranchList {
        default_branch,
        branches,
    })
}

pub(super) async fn fetch_remote_document(
    client: &reqwest::Client,
    connection: &GitHubSyncConnection,
    token: &str,
) -> std::result::Result<Option<RemoteDocument>, ProviderError> {
    let (owner, repo) = parse_github_repo(&connection.repo_url)
        .map_err(|error| ProviderError::Invalid(error.to_string()))?;
    let url = format!(
        "https://api.github.com/repos/{owner}/{repo}/contents/{}",
        connection.path
    );
    let response = client
        .get(url)
        .query(&[("ref", connection.branch.as_str())])
        .header("User-Agent", "qol-tray")
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|error| ProviderError::Upstream(error.to_string()))?;

    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        let body = response.text().await.unwrap_or_default();
        return Err(ProviderError::Auth(format!(
            "GitHub authentication failed: {} {}",
            status, body
        )));
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(ProviderError::Upstream(format!(
            "GitHub returned {}: {}",
            status, body
        )));
    }

    #[derive(Deserialize)]
    struct ResponseBody {
        sha: String,
        content: String,
        encoding: String,
    }

    let body: ResponseBody = response
        .json()
        .await
        .map_err(|error| ProviderError::Upstream(error.to_string()))?;
    if body.encoding != "base64" {
        return Err(ProviderError::Invalid(format!(
            "Unsupported GitHub content encoding: {}",
            body.encoding
        )));
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(body.content.replace('\n', ""))
        .map_err(|error| ProviderError::Invalid(error.to_string()))?;
    let content =
        String::from_utf8(decoded).map_err(|error| ProviderError::Invalid(error.to_string()))?;
    Ok(Some(RemoteDocument {
        revision: body.sha,
        content,
    }))
}

pub(super) async fn push_remote_document(
    client: &reqwest::Client,
    connection: &GitHubSyncConnection,
    token: &str,
    content: &str,
    remote_revision: Option<&str>,
) -> std::result::Result<String, ProviderError> {
    let (owner, repo) = parse_github_repo(&connection.repo_url)
        .map_err(|error| ProviderError::Invalid(error.to_string()))?;
    let url = format!(
        "https://api.github.com/repos/{owner}/{repo}/contents/{}",
        connection.path
    );

    #[derive(Serialize)]
    struct RequestBody<'a> {
        message: &'a str,
        content: String,
        branch: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        sha: Option<&'a str>,
    }

    #[derive(Deserialize)]
    struct ResponseBody {
        content: ContentBody,
    }

    #[derive(Deserialize)]
    struct ContentBody {
        sha: String,
    }

    let response = client
        .put(url)
        .header("User-Agent", "qol-tray")
        .header("Authorization", format!("Bearer {}", token))
        .json(&RequestBody {
            message: &connection.commit_message,
            content: base64::engine::general_purpose::STANDARD.encode(content.as_bytes()),
            branch: &connection.branch,
            sha: remote_revision,
        })
        .send()
        .await
        .map_err(|error| ProviderError::Upstream(error.to_string()))?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        let body = response.text().await.unwrap_or_default();
        return Err(ProviderError::Auth(format!(
            "GitHub authentication failed: {} {}",
            status, body
        )));
    }
    if status == reqwest::StatusCode::CONFLICT {
        let body = response.text().await.unwrap_or_default();
        return Err(ProviderError::Conflict(format!(
            "GitHub reported a sync conflict: {}",
            body
        )));
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(ProviderError::Upstream(format!(
            "GitHub returned {}: {}",
            status, body
        )));
    }

    let body: ResponseBody = response
        .json()
        .await
        .map_err(|error| ProviderError::Upstream(error.to_string()))?;
    Ok(body.content.sha)
}

fn parse_owner_repo(raw: &str) -> Result<(String, String)> {
    let trimmed = raw.trim_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let mut parts = trimmed.split('/');
    let owner = parts.next().unwrap_or_default();
    let repo = parts.next().unwrap_or_default();
    if parts.next().is_some() {
        anyhow::bail!("Repo URL must only include owner and repo");
    }
    if !is_safe_repo_part(owner) || !is_safe_repo_part(repo) {
        anyhow::bail!("Repo URL contains an invalid owner or repo");
    }
    Ok((owner.to_string(), repo.to_string()))
}

fn is_safe_repo_part(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
}

fn is_safe_branch(value: &str) -> bool {
    if value.is_empty() {
        return false;
    }
    if value.contains("..") || value.contains('\\') {
        return false;
    }
    value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '/' || ch == '.')
}
