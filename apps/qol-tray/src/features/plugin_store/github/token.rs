use crate::paths;
use anyhow::Result;
use std::path::PathBuf;

fn token_path() -> Option<PathBuf> {
    paths::github_token_path().ok()
}

#[derive(Debug)]
pub(crate) enum TokenValidationError {
    Empty,
    Invalid(String),
    Upstream(String),
}

impl std::fmt::Display for TokenValidationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(formatter, "Token cannot be empty"),
            Self::Invalid(detail) => write!(formatter, "Invalid token: {}", detail),
            Self::Upstream(detail) => write!(formatter, "GitHub unavailable: {}", detail),
        }
    }
}

impl std::error::Error for TokenValidationError {}

pub(crate) fn build_github_request(
    client: &reqwest::Client,
    url: &str,
    token: Option<&str>,
) -> reqwest::RequestBuilder {
    let mut request = client.get(url).header("User-Agent", "qol-tray");
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {}", token));
    }
    request
}

pub(crate) async fn send_checked(request: reqwest::RequestBuilder) -> Result<reqwest::Response> {
    let response = request.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("GitHub API returned {}: {}", status, body);
    }
    Ok(response)
}

pub(crate) fn get_stored_token() -> Option<String> {
    let path = token_path()?;
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if metadata.file_type().is_symlink() {
        log::warn!("Token file is a symlink, rejecting: {:?}", path);
        return None;
    }

    let token = std::fs::read_to_string(&path).ok()?;
    let token = token.trim();
    if token.is_empty() {
        return None;
    }

    log::info!("Loaded GitHub token from {:?}", path);
    Some(token.to_string())
}

pub(crate) fn store_token(token: &str) -> Result<()> {
    let Some(path) = token_path() else {
        anyhow::bail!("Could not determine token path");
    };
    ensure_token_dir(&path)?;
    crate::file_io::atomic_write(&path, token.trim().as_bytes())?;
    log::info!("Stored GitHub token to {:?}", path);
    Ok(())
}

fn ensure_token_dir(path: &std::path::Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent)?;
    Ok(())
}

pub(crate) async fn validate_token(token: &str) -> std::result::Result<(), TokenValidationError> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        return Err(TokenValidationError::Empty);
    }

    let response = token_validation_response(trimmed).await?;
    token_validation_result(response).await
}

async fn token_validation_response(
    token: &str,
) -> std::result::Result<reqwest::Response, TokenValidationError> {
    reqwest::Client::new()
        .get("https://api.github.com/user")
        .header("User-Agent", "qol-tray")
        .header("Authorization", format!("Bearer {}", token))
        .send()
        .await
        .map_err(|error| TokenValidationError::Upstream(error.to_string()))
}

async fn token_validation_result(
    response: reqwest::Response,
) -> std::result::Result<(), TokenValidationError> {
    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    let body = response.text().await.unwrap_or_default();
    if invalid_status(status) {
        return Err(TokenValidationError::Invalid(format!(
            "{}: {}",
            status, body
        )));
    }
    Err(TokenValidationError::Upstream(format!(
        "{}: {}",
        status, body
    )))
}

fn invalid_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN
}

pub(crate) fn delete_token() -> Result<()> {
    let Some(path) = token_path() else {
        return Ok(());
    };
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}
