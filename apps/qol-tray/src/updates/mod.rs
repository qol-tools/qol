use anyhow::Result;
use serde::Deserialize;
use std::sync::OnceLock;

mod platform;

static LATEST_VERSION: OnceLock<String> = OnceLock::new();

pub(super) const GITHUB_REPO: &str = "qol-tools/qol-tray";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

pub fn latest_version() -> Option<&'static str> {
    LATEST_VERSION.get().map(|s| s.as_str())
}

pub async fn check_for_updates() -> Result<bool> {
    let url = format!(
        "https://api.github.com/repos/{}/releases/latest",
        GITHUB_REPO
    );

    let client = reqwest::Client::new();
    let request = crate::features::plugin_store::github::build_github_request(&client, &url, None);
    let response = crate::features::plugin_store::github::send_checked(request).await?;

    let release: GitHubRelease = response.json().await?;
    let latest = release.tag_name.trim_start_matches('v');

    if is_newer_version(latest, CURRENT_VERSION) {
        let _ = LATEST_VERSION.set(latest.to_string());
        log::info!("Update available: {} -> {}", CURRENT_VERSION, latest);
        return Ok(true);
    }

    log::info!("No updates available (current: {})", CURRENT_VERSION);
    Ok(false)
}

fn is_newer_version(latest: &str, current: &str) -> bool {
    use crate::version::Version;
    Version::parse(latest).is_newer_than(&Version::parse(current))
}

pub async fn download_and_install(events: std::sync::Arc<crate::daemon::EventBus>) -> Result<()> {
    platform::download_and_install(events).await
}
