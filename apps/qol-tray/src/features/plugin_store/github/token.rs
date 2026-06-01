use anyhow::Result;

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
