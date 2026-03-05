use super::DependencyPlan;
use anyhow::Result;
use serde::Deserialize;
use std::path::Path;

pub(super) async fn download_dependency_binary(plan: &DependencyPlan<'_>) -> Result<bool> {
    log::info!("Fetching {} from {}", plan.asset_name, plan.dependency.repo);

    let release = match fetch_latest_release(&plan.dependency.repo).await {
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
        "Release asset '{}' missing for {}",
        plan.asset_name,
        plan.dependency.repo
    );
    Ok(false)
}

fn release_fetch_fallback(plan: &DependencyPlan<'_>, error: &anyhow::Error) -> Result<bool> {
    log::warn!(
        "Failed to fetch release asset {} from {}: {:#}",
        plan.asset_name,
        plan.dependency.repo,
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

async fn fetch_latest_release(repo: &str) -> Result<GitHubRelease> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    let response = github_request(&url).await?;
    Ok(response.json().await?)
}

async fn download_asset(url: &str, dest: &Path) -> Result<()> {
    let response = github_request(url).await?;
    let bytes = response.bytes().await?;
    tokio::fs::write(dest, &bytes).await?;
    Ok(())
}

async fn github_request(url: &str) -> Result<reqwest::Response> {
    let client = reqwest::Client::new();
    let token = super::super::super::github::get_stored_token();
    let request = super::super::super::github::build_github_request(&client, url, token.as_deref());
    super::super::super::github::send_checked(request).await
}
