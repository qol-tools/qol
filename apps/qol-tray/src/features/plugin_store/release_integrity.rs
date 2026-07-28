use std::path::Path;

use anyhow::{Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct GitHubRelease {
    pub(crate) tag_name: String,
    #[serde(default)]
    pub(crate) immutable: bool,
    pub(crate) assets: Vec<GitHubAsset>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct GitHubAsset {
    pub(crate) name: String,
    pub(crate) browser_download_url: String,
    pub(crate) digest: Option<String>,
}

pub(crate) async fn fetch_release(repo: &str, tag: &str) -> Result<GitHubRelease> {
    let url = format!("https://api.github.com/repos/{repo}/releases/tags/{tag}");
    fetch_release_url(&url).await
}

pub(crate) async fn fetch_latest_release(repo: &str) -> Result<GitHubRelease> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    fetch_release_url(&url).await
}

async fn fetch_release_url(url: &str) -> Result<GitHubRelease> {
    let request = super::github::build_github_request(
        &reqwest::Client::new(),
        url,
        crate::credentials::github_bearer_token().as_deref(),
    );
    let response = super::github::send_checked(request).await?;
    Ok(response.json().await?)
}

pub(crate) fn verified_asset(release: &GitHubRelease, asset_name: &str) -> Result<GitHubAsset> {
    require_immutable_release(release)?;
    asset_with_digest(release, asset_name)
}

pub(crate) fn asset_with_digest(release: &GitHubRelease, asset_name: &str) -> Result<GitHubAsset> {
    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .with_context(|| {
            format!(
                "release '{}' is missing asset '{}'",
                release.tag_name, asset_name
            )
        })?;
    expected_sha256(asset)?;
    Ok(asset.clone())
}

pub(crate) fn require_immutable_release(release: &GitHubRelease) -> Result<()> {
    if !release.immutable {
        anyhow::bail!(
            "release '{}' is mutable; enable GitHub release immutability before installing it",
            release.tag_name
        );
    }
    Ok(())
}

pub(crate) async fn download_verified(asset: &GitHubAsset, destination: &Path) -> Result<()> {
    let request = super::github::build_github_request(
        &reqwest::Client::new(),
        &asset.browser_download_url,
        crate::credentials::github_bearer_token().as_deref(),
    );
    let response = super::github::send_checked(request).await?;
    let bytes = response.bytes().await?;
    verify_bytes(asset, &bytes)?;
    qol_fs::atomic_write(destination, &bytes).with_context(|| {
        format!(
            "failed to publish verified release asset at {}",
            destination.display()
        )
    })
}

pub(crate) fn verify_file(asset: &GitHubAsset, path: &Path) -> Result<()> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read downloaded asset {}", path.display()))?;
    verify_bytes(asset, &bytes)
}

fn verify_bytes(asset: &GitHubAsset, bytes: &[u8]) -> Result<()> {
    let expected = expected_sha256(asset)?;
    let actual: [u8; 32] = Sha256::digest(bytes).into();
    if actual != expected {
        anyhow::bail!("SHA-256 mismatch for release asset '{}'", asset.name);
    }
    Ok(())
}

fn expected_sha256(asset: &GitHubAsset) -> Result<[u8; 32]> {
    let digest = asset
        .digest
        .as_deref()
        .with_context(|| format!("release asset '{}' has no digest", asset.name))?;
    let encoded = digest
        .strip_prefix("sha256:")
        .with_context(|| format!("release asset '{}' has an unsupported digest", asset.name))?;
    if encoded.len() != 64 {
        anyhow::bail!("release asset '{}' has an invalid SHA-256", asset.name);
    }
    let mut decoded = [0_u8; 32];
    for (index, pair) in encoded.as_bytes().chunks_exact(2).enumerate() {
        decoded[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Ok(decoded)
}

fn hex_nibble(byte: u8) -> Result<u8> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        b'A'..=b'F' => Ok(byte - b'A' + 10),
        _ => anyhow::bail!("invalid hexadecimal digest"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(immutable: bool, bytes: &[u8]) -> GitHubRelease {
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        GitHubRelease {
            tag_name: "plugin-test-v1.0.0".to_string(),
            immutable,
            assets: vec![GitHubAsset {
                name: "plugin-test-linux-x86_64".to_string(),
                browser_download_url: "https://example.invalid/asset".to_string(),
                digest: Some(digest),
            }],
        }
    }

    #[test]
    fn immutable_release_with_matching_digest_is_accepted() {
        let release = release(true, b"trusted bytes");
        let asset = verified_asset(&release, "plugin-test-linux-x86_64").unwrap();
        verify_bytes(&asset, b"trusted bytes").unwrap();
    }

    #[test]
    fn mutable_release_is_rejected() {
        let error =
            verified_asset(&release(false, b"bytes"), "plugin-test-linux-x86_64").unwrap_err();
        assert!(error.to_string().contains("mutable"));
    }

    #[test]
    fn missing_digest_is_rejected() {
        let mut release = release(true, b"bytes");
        release.assets[0].digest = None;
        let error = verified_asset(&release, "plugin-test-linux-x86_64").unwrap_err();
        assert!(error.to_string().contains("no digest"));
    }

    #[test]
    fn tampered_download_is_rejected() {
        let release = release(true, b"trusted bytes");
        let asset = verified_asset(&release, "plugin-test-linux-x86_64").unwrap();
        let error = verify_bytes(&asset, b"tampered bytes").unwrap_err();
        assert!(error.to_string().contains("mismatch"));
    }
}
