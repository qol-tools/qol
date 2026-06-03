use super::{DependencyPlan, ReleaseTagPick};
use anyhow::Result;
use serde::Deserialize;
use std::path::Path;

pub(super) async fn download_dependency_binary(plan: &DependencyPlan<'_>) -> Result<bool> {
    log::info!(
        "Fetching {} from {} ({:?})",
        plan.asset_name,
        plan.asset_repo(),
        plan.release_tag
    );

    let release = match fetch_release(plan.asset_repo(), &plan.release_tag).await {
        Ok(release) => release,
        Err(error) => return release_fetch_fallback(plan, &error),
    };

    let Some(asset) = find_asset(&release, &plan.asset_name) else {
        return missing_asset(plan);
    };

    download_asset(&asset.browser_download_url, &plan.binary_path).await?;
    Ok(true)
}

fn find_asset<'a>(release: &'a GitHubRelease, asset_name: &str) -> Option<&'a GitHubAsset> {
    release.assets.iter().find(|asset| asset.name == asset_name)
}

fn missing_asset(plan: &DependencyPlan<'_>) -> Result<bool> {
    log::warn!(
        "Release asset '{}' missing for {} ({:?})",
        plan.asset_name,
        plan.asset_repo(),
        plan.release_tag
    );
    Ok(false)
}

fn release_fetch_fallback(plan: &DependencyPlan<'_>, error: &anyhow::Error) -> Result<bool> {
    log::warn!(
        "Failed to fetch release asset {} from {} ({:?}): {:#}",
        plan.asset_name,
        plan.asset_repo(),
        plan.release_tag,
        error
    );
    Ok(false)
}

#[derive(Deserialize)]
struct GitHubRelease {
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

async fn fetch_release(repo: &str, release_tag: &ReleaseTagPick) -> Result<GitHubRelease> {
    let url = release_url(repo, release_tag);
    let response = github_request(&url).await?;
    Ok(response.json().await?)
}

fn release_url(repo: &str, release_tag: &ReleaseTagPick) -> String {
    match release_tag {
        ReleaseTagPick::Latest => {
            format!("https://api.github.com/repos/{}/releases/latest", repo)
        }
        ReleaseTagPick::PluginTag(tag) => {
            format!(
                "https://api.github.com/repos/{}/releases/tags/{}",
                repo, tag
            )
        }
    }
}

async fn download_asset(url: &str, dest: &Path) -> Result<()> {
    let response = github_request(url).await?;
    let bytes = response.bytes().await?;
    tokio::fs::write(dest, &bytes).await?;
    Ok(())
}

async fn github_request(url: &str) -> Result<reqwest::Response> {
    let client = reqwest::Client::new();
    let token = crate::credentials::github_bearer_token();
    let request = super::super::super::github::build_github_request(&client, url, token.as_deref());
    super::super::super::github::send_checked(request).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_url_uses_per_plugin_tag_against_source_repo_not_dep_repo() {
        let cases: &[(&str, ReleaseTagPick, &str)] = &[
            (
                "qol-tools/qol",
                ReleaseTagPick::Latest,
                "https://api.github.com/repos/qol-tools/qol/releases/latest",
            ),
            (
                "qol-tools/qol",
                ReleaseTagPick::PluginTag("plugin-alt-tab-v1.2.3".to_string()),
                "https://api.github.com/repos/qol-tools/qol/releases/tags/plugin-alt-tab-v1.2.3",
            ),
            (
                "qol-tools/qol",
                ReleaseTagPick::PluginTag("plugin-launcher-v0.4.0".to_string()),
                "https://api.github.com/repos/qol-tools/qol/releases/tags/plugin-launcher-v0.4.0",
            ),
        ];
        for (repo, tag, expected) in cases {
            assert_eq!(
                release_url(repo, tag),
                *expected,
                "repo={} tag={:?}",
                repo,
                tag
            );
        }
    }
}
