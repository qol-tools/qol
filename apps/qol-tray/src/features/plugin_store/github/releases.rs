use super::super::release_assets::PlatformTarget;
use super::super::source::{
    required_release_asset_names, select_release_tag, version_from_plugin_tag,
};
use super::catalog::normalized_release_tag;
use super::GitHubClient;
use anyhow::Result;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GitHubRelease {
    pub(super) tag_name: String,
    pub(super) assets: Vec<GitHubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct GitHubAsset {
    pub(super) name: String,
}

impl GitHubClient {
    pub(super) async fn fetch_source_releases(&self) -> Result<Vec<GitHubRelease>> {
        let url = self.source.releases_api_url();
        let response = super::send_checked(self.build_request(&url)).await?;
        let releases: Vec<GitHubRelease> = response.json().await?;
        Ok(releases)
    }

    pub(super) fn select_plugin_version(
        &self,
        plugin_id: &str,
        releases: &[GitHubRelease],
        manifest: &crate::plugins::PluginManifest,
    ) -> Result<String> {
        let target = PlatformTarget::current()?;
        let tags: Vec<&str> = releases.iter().map(|r| r.tag_name.as_str()).collect();
        let Some(tag) = select_release_tag(tags, plugin_id) else {
            anyhow::bail!(
                "no release tag prefixed with '{}-v' found in {}",
                plugin_id,
                self.source.repo
            );
        };
        let release = releases
            .iter()
            .find(|r| r.tag_name == tag)
            .expect("selected tag came from releases list");
        verify_release_assets(plugin_id, release, manifest, target, &self.source.repo)?;
        version_from_plugin_tag(tag, plugin_id)
            .or_else(|| normalized_release_tag(tag).ok())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "release tag '{}' for {} is not valid semver",
                    tag,
                    plugin_id
                )
            })
    }
}

fn verify_release_assets(
    plugin_id: &str,
    release: &GitHubRelease,
    manifest: &crate::plugins::PluginManifest,
    target: PlatformTarget,
    source_repo: &str,
) -> Result<()> {
    let dependencies = manifest
        .dependencies
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("manifest is missing dependencies.binaries"))?;
    if dependencies.binaries.is_empty() {
        anyhow::bail!("manifest has empty dependencies.binaries");
    }
    let names = required_release_asset_names(&dependencies.binaries, target);
    for asset_name in names {
        if !release_has_asset(release, &asset_name) {
            anyhow::bail!(
                "missing asset '{}' in release {} of {} (required by plugin '{}')",
                asset_name,
                release.tag_name,
                source_repo,
                plugin_id
            );
        }
    }
    Ok(())
}

fn release_has_asset(release: &GitHubRelease, asset_name: &str) -> bool {
    release.assets.iter().any(|asset| asset.name == asset_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, asset_names: &[&str]) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.to_string(),
            assets: asset_names
                .iter()
                .map(|n| GitHubAsset {
                    name: (*n).to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn release_has_asset_table() {
        let r = release(
            "plugin-alt-tab-v1.0.0",
            &["alt-tab-linux-x86_64", "alt-tab-macos-aarch64"],
        );
        let cases: &[(&str, bool)] = &[
            ("alt-tab-linux-x86_64", true),
            ("alt-tab-macos-aarch64", true),
            ("alt-tab-windows-x86_64", false),
            ("", false),
        ];
        for (name, expected) in cases {
            assert_eq!(release_has_asset(&r, name), *expected, "asset: {:?}", name);
        }
    }
}
