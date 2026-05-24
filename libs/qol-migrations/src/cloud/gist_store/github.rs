use super::{GistMetadata, GistStore};
use anyhow::{anyhow, Context, Result};
use serde_json::Value;

const GISTS_LIST_URL: &str = "https://api.github.com/gists?per_page=100";
const GIST_BY_ID_URL: &str = "https://api.github.com/gists";
const USER_AGENT: &str = "qol-migrations";
const GITHUB_ACCEPT: &str = "application/vnd.github+json";

pub struct GitHubGistStore {
    http: reqwest::Client,
}

impl GitHubGistStore {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    pub fn with_client(client: reqwest::Client) -> Self {
        Self { http: client }
    }
}

impl Default for GitHubGistStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl GistStore for GitHubGistStore {
    async fn list(&self, token: &str) -> Result<Vec<GistMetadata>> {
        let response = self
            .http
            .get(GISTS_LIST_URL)
            .bearer_auth(token)
            .header(reqwest::header::ACCEPT, GITHUB_ACCEPT)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await
            .context("requesting GitHub gist list")?
            .error_for_status()
            .context("GitHub gist list returned non-success status")?;

        let body: Value = response
            .json()
            .await
            .context("decoding GitHub gist list as JSON")?;

        let entries = body
            .as_array()
            .ok_or_else(|| anyhow!("GitHub gist list response was not a JSON array"))?;

        entries.iter().map(parse_metadata).collect()
    }

    async fn fetch_file(&self, token: &str, gist_id: &str, file_name: &str) -> Result<String> {
        let url = format!("{GIST_BY_ID_URL}/{gist_id}");
        let response = self
            .http
            .get(&url)
            .bearer_auth(token)
            .header(reqwest::header::ACCEPT, GITHUB_ACCEPT)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .send()
            .await
            .with_context(|| format!("requesting GitHub gist {gist_id}"))?
            .error_for_status()
            .with_context(|| format!("GitHub gist {gist_id} returned non-success status"))?;

        let body: Value = response
            .json()
            .await
            .with_context(|| format!("decoding GitHub gist {gist_id} as JSON"))?;

        let file = body
            .get("files")
            .and_then(|files| files.get(file_name))
            .ok_or_else(|| anyhow!("file {file_name} not found in gist {gist_id}"))?;

        let truncated = file
            .get("truncated")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if truncated {
            return Err(anyhow!(
                "file {file_name} in gist {gist_id} is truncated; refusing to return partial content"
            ));
        }

        file.get("content")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| {
                anyhow!("file {file_name} in gist {gist_id} has no string `content` field")
            })
    }
}

fn parse_metadata(entry: &Value) -> Result<GistMetadata> {
    let id = entry
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("gist entry missing string `id`"))?
        .to_owned();

    let description = entry
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();

    let updated_at = entry
        .get("updated_at")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("gist {id} missing string `updated_at`"))?
        .to_owned();

    let public = entry
        .get("public")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let files = entry
        .get("files")
        .and_then(Value::as_object)
        .map(|map| map.keys().cloned().collect())
        .unwrap_or_default();

    Ok(GistMetadata {
        id,
        description,
        files,
        updated_at,
        public,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke() {
        let _ = GitHubGistStore::new();
    }

    #[test]
    fn with_client_accepts_user_provided_client() {
        let client = reqwest::Client::new();
        let _ = GitHubGistStore::with_client(client);
    }

    #[test]
    fn parse_metadata_extracts_fields_and_file_names() {
        let body = serde_json::json!({
            "id": "abc123",
            "description": "hello",
            "updated_at": "2026-05-23T12:00:00Z",
            "public": true,
            "files": {
                "a.txt": { "filename": "a.txt" },
                "b.txt": { "filename": "b.txt" },
            },
        });
        let meta = parse_metadata(&body).unwrap();
        assert_eq!(meta.id, "abc123");
        assert_eq!(meta.description, "hello");
        assert_eq!(meta.updated_at, "2026-05-23T12:00:00Z");
        assert!(meta.public);
        let mut files = meta.files.clone();
        files.sort();
        assert_eq!(files, vec!["a.txt".to_string(), "b.txt".to_string()]);
    }

    #[test]
    fn parse_metadata_defaults_missing_optional_fields() {
        let body = serde_json::json!({
            "id": "abc123",
            "updated_at": "2026-05-23T12:00:00Z",
        });
        let meta = parse_metadata(&body).unwrap();
        assert_eq!(
            meta.description, "",
            "missing description defaults to empty"
        );
        assert!(!meta.public, "missing public defaults to false");
        assert!(meta.files.is_empty(), "missing files defaults to empty");
    }

    #[test]
    fn parse_metadata_errors_on_missing_id() {
        let body = serde_json::json!({ "updated_at": "2026-05-23T12:00:00Z" });
        let err = parse_metadata(&body).unwrap_err();
        assert!(format!("{err}").contains("id"));
    }
}
