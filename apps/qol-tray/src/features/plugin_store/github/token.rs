use anyhow::Result;

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
    crate::credentials::github_bearer_token()
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
