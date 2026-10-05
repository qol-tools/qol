use super::document::{Blob, IndexedRelease, RegistryLocation};
use super::fetch::{get, http_client};
use crate::features::plugin_store::release_integrity;
use anyhow::{Context, Result};
use reqwest::header::WWW_AUTHENTICATE;
use reqwest::{StatusCode, Url};
use serde::Deserialize;
use std::path::Path;

pub(crate) async fn download_asset(
    release: &IndexedRelease,
    asset_name: &str,
    destination: &Path,
) -> Result<bool> {
    let Some(blob) = release.files.assets.get(asset_name) else {
        return Ok(false);
    };
    let bytes = fetch_blob(&http_client(), &release.registry, blob, asset_name).await?;
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
    let mut response = send(client.get(&url), &url).await?;
    if response.status() == StatusCode::UNAUTHORIZED {
        let token = anonymous_token(client, registry, &response).await?;
        response = send(client.get(&url).bearer_auth(token), &url).await?;
    }
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("{url} answered {status}");
    }
    let bytes = response.bytes().await?;
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
        .with_context(|| format!("could not reach {url}"))
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

#[derive(Debug, PartialEq, Eq)]
struct BearerChallenge {
    realm: String,
    service: Option<String>,
    scope: Option<String>,
}

impl BearerChallenge {
    fn parse(header: &str) -> Result<Self> {
        let (scheme, params) = header.trim().split_once(' ').unwrap_or((header, ""));
        if !scheme.eq_ignore_ascii_case("bearer") {
            anyhow::bail!("the registry asked for {scheme} credentials");
        }
        let mut realm = None;
        let mut service = None;
        let mut scope = None;
        for (key, value) in challenge_params(params) {
            match key.as_str() {
                "realm" => realm = Some(value),
                "service" => service = Some(value),
                "scope" => scope = Some(value),
                _ => {}
            }
        }
        Ok(Self {
            realm: realm.context("the registry bearer challenge has no realm")?,
            service,
            scope,
        })
    }

    fn token_url(&self, registry: &RegistryLocation) -> Result<Url> {
        let mut url = Url::parse(&self.realm).context("the registry token realm is not a URL")?;
        let registry_url = Url::parse(&registry.url).context("the registry URL is invalid")?;
        if url.scheme() != "https" && url.origin() != registry_url.origin() {
            anyhow::bail!("refusing a non-HTTPS token realm {}", self.realm);
        }
        {
            let mut query = url.query_pairs_mut();
            if let Some(service) = &self.service {
                query.append_pair("service", service);
            }
            if let Some(scope) = &self.scope {
                query.append_pair("scope", scope);
            }
        }
        Ok(url)
    }
}

fn challenge_params(params: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut rest = params.trim();
    while let Some((key, after_key)) = rest.split_once('=') {
        let (value, after_value) = challenge_value(after_key.trim_start());
        pairs.push((key.trim().to_ascii_lowercase(), value));
        rest = after_value
            .trim_start()
            .trim_start_matches(',')
            .trim_start();
    }
    pairs
}

fn challenge_value(input: &str) -> (String, &str) {
    let Some(quoted) = input.strip_prefix('"') else {
        let end = input.find(',').unwrap_or(input.len());
        return (input[..end].trim().to_string(), &input[end..]);
    };
    let mut value = String::new();
    let mut chars = quoted.char_indices();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '"' => return (value, &quoted[index + 1..]),
            '\\' => value.extend(chars.next().map(|(_, escaped)| escaped)),
            _ => value.push(ch),
        }
    }
    (value, "")
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
    fn parses_bearer_challenges_with_quoted_commas() {
        let cases: &[(&str, BearerChallenge)] = &[
            (
                r#"Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:qol-tools/plugins:pull""#,
                BearerChallenge {
                    realm: "https://ghcr.io/token".to_string(),
                    service: Some("ghcr.io".to_string()),
                    scope: Some("repository:qol-tools/plugins:pull".to_string()),
                },
            ),
            (
                r#"bearer realm="https://auth.example/token", scope="repository:a/b:pull,push""#,
                BearerChallenge {
                    realm: "https://auth.example/token".to_string(),
                    service: None,
                    scope: Some("repository:a/b:pull,push".to_string()),
                },
            ),
        ];
        for (header, expected) in cases {
            assert_eq!(
                &BearerChallenge::parse(header).unwrap(),
                expected,
                "{header}"
            );
        }
        assert!(BearerChallenge::parse(r#"Basic realm="x""#).is_err());
        assert!(BearerChallenge::parse(r#"Bearer service="ghcr.io""#).is_err());
    }

    #[test]
    fn token_url_carries_service_and_scope_and_refuses_plain_http_elsewhere() {
        let challenge = BearerChallenge::parse(
            r#"Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:qol-tools/plugins:pull""#,
        )
        .unwrap();
        let url = challenge.token_url(&registry("https://ghcr.io")).unwrap();
        assert_eq!(
            url.as_str(),
            "https://ghcr.io/token?service=ghcr.io&scope=repository%3Aqol-tools%2Fplugins%3Apull"
        );

        let local =
            BearerChallenge::parse(r#"Bearer realm="http://127.0.0.1:5000/token""#).unwrap();
        assert!(local.token_url(&registry("http://127.0.0.1:5000")).is_ok());
        assert!(local.token_url(&registry("https://ghcr.io")).is_err());
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
