use super::document::{Blob, IndexedRelease, RegistryLocation};
use super::fetch::{blob_client, get, read_body};
use crate::features::plugin_store::release_integrity;
use anyhow::{Context, Result};
use qol_plugin_index::BearerChallenge;
use reqwest::header::WWW_AUTHENTICATE;
use reqwest::StatusCode;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

static PULL_TOKENS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Mutex::default);

pub(crate) async fn download_asset(
    release: &IndexedRelease,
    asset_name: &str,
    destination: &Path,
) -> Result<bool> {
    let Some(blob) = release.files.assets.get(asset_name) else {
        return Ok(false);
    };
    let bytes = fetch_blob(&blob_client(), &release.registry, blob, asset_name).await?;
    qol_fs::atomic_write(destination, &bytes)
        .with_context(|| format!("failed to write {asset_name} to {}", destination.display()))?;
    Ok(true)
}

pub(super) async fn fetch_blob(
    client: &reqwest::Client,
    registry: &RegistryLocation,
    blob: &Blob,
    label: &str,
) -> Result<Vec<u8>> {
    let url = blob_url(registry, &blob.digest);
    let scope = format!("{}/{}", registry.url, registry.repository);
    let mut request = client.get(&url);
    if let Some(token) = remembered_token(&scope) {
        request = request.bearer_auth(token);
    }
    let mut response = send(request, &url).await?;
    if response.status() == StatusCode::UNAUTHORIZED {
        let token = anonymous_token(client, registry, &response)
            .await
            .map_err(|error| anyhow::anyhow!("{url} refused an anonymous pull: {error:#}"))?;
        remember_token(&scope, &token);
        response = send(client.get(&url).bearer_auth(token), &url).await?;
    }
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!("{url} answered {status}"));
    }
    let bytes = read_body(&url, response.bytes()).await?;
    if bytes.len() as u64 != blob.size {
        anyhow::bail!(
            "{label} is {} bytes but the plugin index says {}",
            bytes.len(),
            blob.size
        );
    }
    release_integrity::verify_sha256(&blob.digest, &bytes, label)?;
    qol_runtime::probe!(
        "PLUGIN_INDEX",
        "event=blob_verified name={} digest={} bytes={}",
        label,
        blob.digest,
        bytes.len()
    );
    Ok(bytes.to_vec())
}

fn blob_url(registry: &RegistryLocation, digest: &str) -> String {
    format!(
        "{}/v2/{}/blobs/{}",
        registry.url.trim_end_matches('/'),
        registry.repository,
        digest
    )
}

async fn send(request: reqwest::RequestBuilder, url: &str) -> Result<reqwest::Response> {
    request
        .send()
        .await
        .map_err(|error| anyhow::anyhow!("could not reach {url}: {error}"))
}

fn remembered_token(scope: &str) -> Option<String> {
    PULL_TOKENS.lock().ok()?.get(scope).cloned()
}

fn remember_token(scope: &str, token: &str) {
    if let Ok(mut tokens) = PULL_TOKENS.lock() {
        tokens.insert(scope.to_string(), token.to_string());
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    token: Option<String>,
    access_token: Option<String>,
}

async fn anonymous_token(
    client: &reqwest::Client,
    registry: &RegistryLocation,
    response: &reqwest::Response,
) -> Result<String> {
    let header = response
        .headers()
        .get(WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
        .context("the registry asked for credentials without a bearer challenge")?;
    let challenge = BearerChallenge::parse(header)?;
    let token_url = challenge.token_url(registry)?;
    let body: TokenResponse = get(client, token_url.as_str()).await?.json().await?;
    body.token
        .or(body.access_token)
        .context("the registry token response has no token")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(url: &str) -> RegistryLocation {
        RegistryLocation {
            url: url.to_string(),
            repository: "qol-tools/plugins".to_string(),
        }
    }

    #[test]
    fn blob_url_joins_registry_repository_and_digest() {
        let cases: &[(&str, &str)] = &[
            (
                "https://ghcr.io",
                "https://ghcr.io/v2/qol-tools/plugins/blobs/sha256:ab",
            ),
            (
                "https://ghcr.io/",
                "https://ghcr.io/v2/qol-tools/plugins/blobs/sha256:ab",
            ),
        ];
        for (url, expected) in cases {
            assert_eq!(blob_url(&registry(url), "sha256:ab"), *expected);
        }
    }
}
