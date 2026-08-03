use super::{GistMetadata, GistStore};
use anyhow::{anyhow, Context, Result};
use serde_json::Value;

const API_BASE_URL: &str = "https://api.github.com";
const USER_AGENT: &str = "qol-migrations";
const GITHUB_ACCEPT: &str = "application/vnd.github+json";
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub struct GitHubGistStore {
    http: reqwest::Client,
    base_url: String,
}

impl GitHubGistStore {
    pub fn new() -> Result<Self> {
        Self::with_timeout(REQUEST_TIMEOUT, API_BASE_URL)
    }

    pub fn with_client(client: reqwest::Client) -> Self {
        Self {
            http: client,
            base_url: API_BASE_URL.to_string(),
        }
    }

    fn with_timeout(timeout: std::time::Duration, base_url: impl Into<String>) -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .map_err(|error| anyhow!("building GitHub API client: {error}"))?,
            base_url: base_url.into(),
        })
    }
}

#[async_trait::async_trait]
impl GistStore for GitHubGistStore {
    async fn list(&self, token: &str) -> Result<Vec<GistMetadata>> {
        let response = self
            .http
            .get(format!("{}/gists?per_page=100", self.base_url))
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
        let url = format!("{}/gists/{gist_id}", self.base_url);
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
        let _ = GitHubGistStore::new().unwrap();
    }

    #[test]
    fn with_client_accepts_user_provided_client() {
        let client = reqwest::Client::new();
        let _ = GitHubGistStore::with_client(client);
    }

    #[test]
    fn list_times_out_when_server_stalls() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(15)))
                .unwrap();
            let mut buf = [0u8; 1024];
            loop {
                match std::io::Read::read(&mut stream, &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => continue,
                }
            }
        });
        let store = GitHubGistStore::with_timeout(
            std::time::Duration::from_millis(300),
            format!("http://{addr}"),
        )
        .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let started = std::time::Instant::now();
        let result = runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), store.list("token")).await
        });
        let elapsed = started.elapsed();
        drop(runtime);
        server.join().unwrap();
        assert!(
            result.is_ok(),
            "stalled request must fail via the client timeout, got: {result:?}"
        );
        assert!(
            elapsed >= std::time::Duration::from_millis(200),
            "request failed too fast to prove the server stalled"
        );
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "client timeout did not bound the stalled request: {elapsed:?}"
        );
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
