use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use super::{ProviderError, RemoteDocument};
use crate::features::profile::sync::GitHubSyncConnection;

const PROFILE_FILENAME: &str = "qol-tray-profile.json";
const GIST_DESCRIPTION: &str = "QoL Tray Profile Sync";

pub(super) fn validate_connection(connection: &GitHubSyncConnection) -> Result<()> {
    if connection.gist_id.is_empty() {
        anyhow::bail!("Gist ID is required");
    }
    if !is_safe_gist_id(&connection.gist_id) {
        anyhow::bail!("Invalid gist ID");
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
        .ok_or_else(|| anyhow!("GitHub account is not connected"))
}

pub(crate) async fn ensure_profile_gist(
    client: &reqwest::Client,
    token: &str,
) -> std::result::Result<String, ProviderError> {
    if let Some(gist_id) = find_profile_gist(client, token).await? {
        return Ok(gist_id);
    }
    create_profile_gist(client, token).await
}

pub(super) async fn fetch_remote_document(
    client: &reqwest::Client,
    connection: &GitHubSyncConnection,
    token: &str,
) -> std::result::Result<Option<RemoteDocument>, ProviderError> {
    let url = format!("https://api.github.com/gists/{}", connection.gist_id);
    let response = client
        .get(url)
        .header("User-Agent", "qol-tray")
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|error| ProviderError::Transport(error.to_string()))?;

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

    let body: GistResponse = response
        .json()
        .await
        .map_err(|error| ProviderError::Upstream(error.to_string()))?;

    let Some(file) = body.files.get(PROFILE_FILENAME) else {
        return Ok(None);
    };
    let content = file.content.clone().unwrap_or_default();
    let revision = gist_revision(&body.history);
    Ok(Some(RemoteDocument { revision, content }))
}

pub(super) async fn push_remote_document(
    client: &reqwest::Client,
    connection: &GitHubSyncConnection,
    token: &str,
    content: &str,
    remote_revision: Option<&str>,
) -> std::result::Result<String, ProviderError> {
    if let Some(expected) = remote_revision {
        let current = fetch_current_revision(client, &connection.gist_id, token).await?;
        if current != expected {
            return Err(ProviderError::Conflict(
                "Gist was modified since last sync".to_string(),
            ));
        }
    }

    let url = format!("https://api.github.com/gists/{}", connection.gist_id);
    let response = client
        .patch(url)
        .header("User-Agent", "qol-tray")
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "files": {
                PROFILE_FILENAME: {
                    "content": content,
                }
            }
        }))
        .send()
        .await
        .map_err(|error| ProviderError::Transport(error.to_string()))?;

    let status = response.status();
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

    let body: GistResponse = response
        .json()
        .await
        .map_err(|error| ProviderError::Upstream(error.to_string()))?;
    Ok(gist_revision(&body.history))
}

async fn find_profile_gist(
    client: &reqwest::Client,
    token: &str,
) -> std::result::Result<Option<String>, ProviderError> {
    for page in 1..=3 {
        let response = client
            .get("https://api.github.com/gists")
            .query(&[("per_page", "100"), ("page", &page.to_string())])
            .header("User-Agent", "qol-tray")
            .header("Authorization", format!("Bearer {}", token))
            .send()
            .await
            .map_err(|error| ProviderError::Transport(error.to_string()))?;

        let gists: Vec<GistListEntry> = parse_github_response(response).await?;
        if gists.is_empty() {
            break;
        }
        for gist in &gists {
            if gist.files.contains_key(PROFILE_FILENAME) {
                return Ok(Some(gist.id.clone()));
            }
        }
    }
    Ok(None)
}

async fn create_profile_gist(
    client: &reqwest::Client,
    token: &str,
) -> std::result::Result<String, ProviderError> {
    #[derive(Serialize)]
    struct RequestBody {
        description: &'static str,
        public: bool,
        files: std::collections::HashMap<&'static str, FileContent>,
    }

    #[derive(Serialize)]
    struct FileContent {
        content: &'static str,
    }

    let mut files = std::collections::HashMap::new();
    files.insert(PROFILE_FILENAME, FileContent { content: "{}" });

    let response = client
        .post("https://api.github.com/gists")
        .header("User-Agent", "qol-tray")
        .header("Authorization", format!("Bearer {}", token))
        .json(&RequestBody {
            description: GIST_DESCRIPTION,
            public: false,
            files,
        })
        .send()
        .await
        .map_err(|error| ProviderError::Transport(error.to_string()))?;

    let body: GistResponse = parse_github_response(response).await?;
    Ok(body.id)
}

async fn fetch_current_revision(
    client: &reqwest::Client,
    gist_id: &str,
    token: &str,
) -> std::result::Result<String, ProviderError> {
    let url = format!("https://api.github.com/gists/{gist_id}");
    let response = client
        .get(url)
        .header("User-Agent", "qol-tray")
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|error| ProviderError::Transport(error.to_string()))?;

    let body: GistResponse = parse_github_response(response).await?;
    Ok(gist_revision(&body.history))
}

#[derive(Deserialize)]
struct GistResponse {
    id: String,
    files: std::collections::HashMap<String, GistFile>,
    #[serde(default)]
    history: Vec<GistHistoryEntry>,
}

#[derive(Deserialize)]
struct GistFile {
    content: Option<String>,
}

#[derive(Deserialize)]
struct GistHistoryEntry {
    version: String,
}

#[derive(Deserialize)]
struct GistListEntry {
    id: String,
    files: std::collections::HashMap<String, serde_json::Value>,
}

fn gist_revision(history: &[GistHistoryEntry]) -> String {
    history
        .first()
        .map(|entry| entry.version.clone())
        .unwrap_or_default()
}

async fn parse_github_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> std::result::Result<T, ProviderError> {
    let status = response.status();
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
    response
        .json()
        .await
        .map_err(|error| ProviderError::Upstream(error.to_string()))
}

fn is_safe_gist_id(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|ch| ch.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(200))]

        #[test]
        fn prop_safe_gist_id_accepts_hex(id in "[0-9a-fA-F]{1,40}") {
            assert!(is_safe_gist_id(&id));
        }

        #[test]
        fn prop_safe_gist_id_rejects_non_hex(id in "[^0-9a-fA-F]+") {
            assert!(!is_safe_gist_id(&id));
        }

        #[test]
        fn prop_validate_connection_rejects_empty_gist_id(
            pull in proptest::bool::ANY,
            push in proptest::bool::ANY,
        ) {
            let connection = GitHubSyncConnection {
                gist_id: String::new(),
                pull_on_launch: pull,
                push_on_change: push,
            };
            assert!(validate_connection(&connection).is_err());
        }

        #[test]
        fn prop_validate_connection_accepts_valid_gist_id(
            id in "[0-9a-f]{20,40}",
            pull in proptest::bool::ANY,
            push in proptest::bool::ANY,
        ) {
            let connection = GitHubSyncConnection {
                gist_id: id,
                pull_on_launch: pull,
                push_on_change: push,
            };
            assert!(validate_connection(&connection).is_ok());
        }
    }

    #[test]
    fn safe_gist_id_edge_cases() {
        let cases = [
            ("", false),
            ("a", true),
            ("0", true),
            ("abc123def456", true),
            ("ABC123", true),
            ("abc 123", false),
            ("abc\n", false),
            ("abc/def", false),
            ("abc..def", false),
            ("ghijkl", false), // g-z are not hex
        ];
        for (input, expected) in cases {
            assert_eq!(is_safe_gist_id(input), expected, "input: {input:?}");
        }
    }
}
