use anyhow::Result;
use std::io::Write;
use std::path::Path;

use crate::daemon::{DaemonEvent, EventBus};
use crate::features::plugin_store::release_integrity::{
    self, GitHubAsset, GitHubRelease, IndexedHostAsset, IndexedHostRelease,
};

use super::super::{FoundUpdate, GITHUB_REPO, HOST_TAG_PREFIX};

#[cfg(feature = "dev")]
pub(super) fn dev_update_url() -> Option<String> {
    std::env::var("QOL_TRAY_DEV_UPDATE_URL").ok()
}

#[cfg(not(feature = "dev"))]
pub(super) fn dev_update_url() -> Option<String> {
    None
}

pub(super) struct HostUpdate {
    version: String,
    files: HostFiles,
}

enum HostFiles {
    PluginIndex(IndexedHostRelease),
    GitHubRelease(GitHubRelease),
}

pub(super) enum HostAsset {
    PluginIndex(IndexedHostAsset),
    GitHubRelease(GitHubAsset),
}

pub(super) async fn resolve_update(update: Option<FoundUpdate>) -> Result<HostUpdate> {
    let update = update.ok_or_else(|| anyhow::anyhow!("No update version available"))?;
    let version = update.version().to_string();
    let files = match update {
        FoundUpdate::PluginIndex(release) => HostFiles::PluginIndex(release),
        FoundUpdate::GitHubRelease { version } => {
            let tag = format!("{HOST_TAG_PREFIX}-v{version}");
            HostFiles::GitHubRelease(release_integrity::fetch_release(GITHUB_REPO, &tag).await?)
        }
    };
    Ok(HostUpdate { version, files })
}

impl HostUpdate {
    pub(super) fn version(&self) -> &str {
        &self.version
    }

    pub(super) fn asset(&self, name: &str) -> Result<HostAsset> {
        match &self.files {
            HostFiles::PluginIndex(release) => release.asset(name).map(HostAsset::PluginIndex),
            HostFiles::GitHubRelease(release) => {
                release_integrity::verified_asset(release, name).map(HostAsset::GitHubRelease)
            }
        }
    }
}

impl HostAsset {
    fn location(&self) -> String {
        match self {
            Self::PluginIndex(asset) => asset.location(),
            Self::GitHubRelease(asset) => asset.browser_download_url.clone(),
        }
    }

    pub(super) async fn download(&self, dest: &Path, on_percent: impl FnMut(u8)) -> Result<()> {
        match self {
            Self::PluginIndex(asset) => {
                write_response(asset.open().await?, dest, on_percent).await?;
                asset.verify_file(dest)
            }
            Self::GitHubRelease(asset) => {
                download_to(&asset.browser_download_url, dest, on_percent).await?;
                release_integrity::verify_file(asset, dest)
            }
        }
    }
}

pub(super) async fn download_update_asset(
    update: Option<&HostUpdate>,
    dev_url: Option<String>,
    asset_name: &str,
    dest: &Path,
    events: &EventBus,
) -> Result<()> {
    let on_percent = |percent| {
        events.send(DaemonEvent::UpdateProgress { percent });
        super::super::record_update_progress(percent);
    };
    if let Some(url) = dev_url {
        log::info!("Downloading update from {}", url);
        return download_to(&url, dest, on_percent).await;
    }
    let update = update.ok_or_else(|| anyhow::anyhow!("No verified update asset available"))?;
    let asset = update.asset(asset_name)?;
    log::info!("Downloading update from {}", asset.location());
    asset.download(dest, on_percent).await
}

async fn download_to(url: &str, dest: &Path, on_percent: impl FnMut(u8)) -> Result<()> {
    let request = crate::features::plugin_store::github::build_github_request(
        &crate::features::plugin_store::blob_client(),
        url,
        None,
    );
    let response = crate::features::plugin_store::github::send_checked(request).await?;
    write_response(response, dest, on_percent).await
}

async fn write_response(
    response: reqwest::Response,
    dest: &Path,
    mut on_percent: impl FnMut(u8),
) -> Result<()> {
    let mut body = crate::features::connection_health::observe_body(response);
    let total = body.content_length();
    let mut file = std::fs::File::create(dest)?;
    let mut downloaded: u64 = 0;
    let mut last_percent: u8 = 0;
    while let Some(chunk) = body.chunk().await? {
        let chunk = chunk.as_ref();
        file.write_all(chunk)?;
        downloaded += chunk.len() as u64;
        let percent = total
            .map(|t| ((downloaded * 100) / t).min(100) as u8)
            .unwrap_or(0);
        if percent != last_percent {
            on_percent(percent);
            last_percent = percent;
        }
    }
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    const HOST: &str = "qol-tray-host.exe";
    const INSTALLER: &str = "qol-tray-install.exe";

    fn github(immutable: bool, assets: &[(&str, Option<&[u8]>)]) -> HostUpdate {
        let assets = assets
            .iter()
            .map(|(name, bytes)| GitHubAsset {
                name: name.to_string(),
                browser_download_url: format!("https://github.invalid/{name}"),
                digest: bytes.map(|bytes| format!("sha256:{:x}", Sha256::digest(bytes))),
            })
            .collect();
        HostUpdate {
            version: "3.2.0".to_string(),
            files: HostFiles::GitHubRelease(GitHubRelease {
                tag_name: "qol-tray-v3.2.0".to_string(),
                immutable,
                assets,
            }),
        }
    }

    fn indexed(assets: &[(&str, &[u8])]) -> HostUpdate {
        HostUpdate {
            version: "3.2.0".to_string(),
            files: HostFiles::PluginIndex(IndexedHostRelease::test_fixture("3.2.0", assets)),
        }
    }

    #[test]
    fn each_asset_comes_from_the_source_that_found_the_update() {
        let cases: Vec<(&str, HostUpdate, &str, Result<&str, &str>)> = vec![
            (
                "index lists the asset",
                indexed(&[(HOST, b"host"), (INSTALLER, b"installer")]),
                INSTALLER,
                Ok("https://ghcr.io/v2/qol-tools/plugins/blobs/sha256:"),
            ),
            (
                "index does not list the asset",
                indexed(&[(HOST, b"host")]),
                INSTALLER,
                Err("lists no qol-tray-install.exe"),
            ),
            (
                "immutable release with a digest",
                github(true, &[(HOST, Some(b"host")), (INSTALLER, Some(b"i"))]),
                INSTALLER,
                Ok("https://github.invalid/qol-tray-install.exe"),
            ),
            (
                "mutable release",
                github(false, &[(HOST, Some(b"host"))]),
                HOST,
                Err("mutable"),
            ),
            (
                "release asset without a digest",
                github(true, &[(HOST, None)]),
                HOST,
                Err("no digest"),
            ),
            (
                "release without the asset",
                github(true, &[(HOST, Some(b"host"))]),
                INSTALLER,
                Err("missing asset"),
            ),
        ];
        for (case, update, name, expected) in cases {
            let resolved = update.asset(name);
            match (resolved, expected) {
                (Ok(asset), Ok(location)) => {
                    assert!(asset.location().starts_with(location), "{case}")
                }
                (Err(error), Err(cause)) => {
                    assert!(format!("{error:#}").contains(cause), "{case}: {error:#}")
                }
                (Ok(asset), Err(_)) => panic!("{case}: resolved {}", asset.location()),
                (Err(error), Ok(_)) => panic!("{case}: {error:#}"),
            }
        }
    }

    #[test]
    fn an_index_sourced_asset_never_reads_github() {
        let update = indexed(&[(HOST, b"host")]);
        let asset = update.asset(HOST).unwrap();
        assert!(matches!(asset, HostAsset::PluginIndex(_)));
        assert!(!asset.location().contains("github"));
    }
}
