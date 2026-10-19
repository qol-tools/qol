use super::document::{Blob, IndexedRelease, RegistryLocation};
use super::fetch::{blob_client, get, read_body};
use crate::features::connection_health::SendObserved;
use crate::features::plugin_store::release_integrity;
use anyhow::{Context, Result};
use qol_plugin_index::{BearerChallenge, INDEX_TAG, SIGNED_INDEX_MEDIA_TYPE};
use reqwest::header::{ACCEPT, WWW_AUTHENTICATE};
use reqwest::StatusCode;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{LazyLock, Mutex};

static PULL_TOKENS: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Mutex::default);

const OCI_MANIFEST_MEDIA_TYPE: &str = "application/vnd.oci.image.manifest.v1+json";

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

pub(super) async fn fetch_signed_index(
    client: &reqwest::Client,
    registry: &RegistryLocation,
) -> Result<Vec<u8>> {
    let url = manifest_url(registry, INDEX_TAG);
    let response = pull(client, registry, &url, Some(OCI_MANIFEST_MEDIA_TYPE)).await?;
    let manifest: Manifest = read_body(&url, response.json()).await?;
    let layer = signed_index_layer(manifest)?;
    fetch_blob(client, registry, &layer, "the signed plugin index").await
}

#[derive(Deserialize)]
struct Manifest {
    layers: Vec<Descriptor>,
}

#[derive(Deserialize)]
struct Descriptor {
    #[serde(rename = "mediaType")]
    media_type: String,
    digest: String,
    size: u64,
}

fn signed_index_layer(manifest: Manifest) -> Result<Blob> {
    let mut layers = manifest
        .layers
        .into_iter()
        .filter(|layer| layer.media_type == SIGNED_INDEX_MEDIA_TYPE);
    let (Some(layer), None) = (layers.next(), layers.next()) else {
        anyhow::bail!(
            "the {INDEX_TAG} manifest does not hold exactly one signed plugin index layer"
        );
    };
    Ok(Blob {
        digest: layer.digest,
        size: layer.size,
    })
}

pub(super) async fn fetch_blob(
    client: &reqwest::Client,
    registry: &RegistryLocation,
    blob: &Blob,
    label: &str,
) -> Result<Vec<u8>> {
    let url = blob_url(registry, &blob.digest);
    let response = pull(client, registry, &url, None).await?;
    let bytes = read_body(&url, response.bytes()).await?;
    verify_blob(blob, &bytes, label)?;
    qol_runtime::probe!(
        "PLUGIN_INDEX",
        "event=blob_verified name={} digest={} bytes={}",
        label,
        blob.digest,
        bytes.len()
    );
    Ok(bytes.to_vec())
}

pub(super) async fn open_blob(
    registry: &RegistryLocation,
    blob: &Blob,
) -> Result<reqwest::Response> {
    pull(
        &blob_client(),
        registry,
        &blob_url(registry, &blob.digest),
        None,
    )
    .await
}

pub(super) fn verify_blob(blob: &Blob, bytes: &[u8], label: &str) -> Result<()> {
    if bytes.len() as u64 != blob.size {
        anyhow::bail!(
            "{label} is {} bytes but the plugin index says {}",
            bytes.len(),
            blob.size
        );
    }
    release_integrity::verify_sha256(&blob.digest, bytes, label)
}

async fn pull(
    client: &reqwest::Client,
    registry: &RegistryLocation,
    url: &str,
    accept: Option<&str>,
) -> Result<reqwest::Response> {
    let scope = format!("{}/{}", registry.url, registry.repository);
    let request = |token: Option<&str>| {
        let mut request = client.get(url);
        if let Some(accept) = accept {
            request = request.header(ACCEPT, accept);
        }
        match token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
    };
    let mut response = send(request(remembered_token(&scope).as_deref()), url).await?;
    if response.status() == StatusCode::UNAUTHORIZED {
        let token = anonymous_token(client, registry, &response)
            .await
            .map_err(|error| anyhow::anyhow!("{url} refused an anonymous pull: {error:#}"))?;
        remember_token(&scope, &token);
        response = send(request(Some(&token)), url).await?;
    }
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow::anyhow!("{url} answered {status}"));
    }
    Ok(response)
}

fn manifest_url(registry: &RegistryLocation, reference: &str) -> String {
    format!(
        "{}/v2/{}/manifests/{}",
        registry.url.trim_end_matches('/'),
        registry.repository,
        reference
    )
}

pub(super) fn blob_url(registry: &RegistryLocation, digest: &str) -> String {
    format!(
        "{}/v2/{}/blobs/{}",
        registry.url.trim_end_matches('/'),
        registry.repository,
        digest
    )
}

async fn send(request: reqwest::RequestBuilder, url: &str) -> Result<reqwest::Response> {
    request
        .send_observed()
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
        assert_eq!(
            manifest_url(&registry("https://ghcr.io/"), INDEX_TAG),
            "https://ghcr.io/v2/qol-tools/plugins/manifests/index"
        );
    }

    #[test]
    fn the_index_manifest_must_hold_exactly_one_signed_index_layer() {
        let layer = |media_type: &str, digest: &str| serde_json::json!({ "mediaType": media_type, "digest": digest, "size": 7 });
        let signed = SIGNED_INDEX_MEDIA_TYPE;
        let other = "application/octet-stream";
        let cases: Vec<(&str, Vec<serde_json::Value>, Option<&str>)> = vec![
            (
                "one signed layer",
                vec![layer(signed, "sha256:a")],
                Some("sha256:a"),
            ),
            (
                "signed layer beside another",
                vec![layer(other, "sha256:b"), layer(signed, "sha256:a")],
                Some("sha256:a"),
            ),
            ("no layers", vec![], None),
            ("only other layers", vec![layer(other, "sha256:b")], None),
            (
                "two signed layers",
                vec![layer(signed, "sha256:a"), layer(signed, "sha256:c")],
                None,
            ),
        ];
        for (case, layers, expected) in cases {
            let manifest: Manifest =
                serde_json::from_value(serde_json::json!({ "layers": layers })).unwrap();
            let picked = signed_index_layer(manifest).ok();
            assert_eq!(
                picked.as_ref().map(|blob| blob.digest.as_str()),
                expected,
                "{case}"
            );
            if let Some(blob) = picked {
                assert_eq!(blob.size, 7, "{case}");
            }
        }
    }
}
