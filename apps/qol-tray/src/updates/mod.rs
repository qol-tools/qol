use anyhow::Result;
use serde::Deserialize;
use std::sync::OnceLock;

pub(crate) mod platform;

static LATEST_VERSION: OnceLock<String> = OnceLock::new();

pub(super) const GITHUB_REPO: &str = "qol-tools/qol";
pub(super) const HOST_TAG_PREFIX: &str = "qol-tray";
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
        "https://api.github.com/repos/{}/releases?per_page=100",
        GITHUB_REPO
    );

    let client = reqwest::Client::new();
    let request = crate::features::plugin_store::github::build_github_request(&client, &url, None);
    let response = crate::features::plugin_store::github::send_checked(request).await?;

    let releases: Vec<GitHubRelease> = response.json().await?;
    let Some(latest) = pick_latest_host_version(&releases) else {
        log::info!("No qol-tray-v* releases published yet (current: {CURRENT_VERSION})");
        return Ok(false);
    };

    if is_newer_version(&latest, CURRENT_VERSION) {
        let _ = LATEST_VERSION.set(latest.clone());
        log::info!("Update available: {} -> {}", CURRENT_VERSION, latest);
        return Ok(true);
    }

    log::info!("No updates available (current: {})", CURRENT_VERSION);
    Ok(false)
}

fn pick_latest_host_version(releases: &[GitHubRelease]) -> Option<String> {
    use crate::features::plugin_store::source::{select_release_tag, version_from_plugin_tag};
    let tag = select_release_tag(releases.iter().map(|r| r.tag_name.as_str()), HOST_TAG_PREFIX)?;
    version_from_plugin_tag(tag, HOST_TAG_PREFIX)
}

fn is_newer_version(latest: &str, current: &str) -> bool {
    use crate::version::Version;
    Version::parse(latest).is_newer_than(&Version::parse(current))
}

pub async fn download_and_install(events: std::sync::Arc<crate::daemon::EventBus>) -> Result<()> {
    platform::download_and_install(events).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tag: &str) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.to_string(),
        }
    }

    #[test]
    fn pick_latest_host_version_strips_qol_tray_prefix() {
        let cases = [
            (vec!["qol-tray-v1.2.3"], Some("1.2.3")),
            (vec!["qol-tray-v0.6.0"], Some("0.6.0")),
            (vec!["qol-tray-v9.9.9-beta.1"], Some("9.9.9-beta.1")),
        ];
        for (tags, expected) in cases {
            let releases: Vec<_> = tags.iter().map(|t| rel(t)).collect();
            assert_eq!(
                pick_latest_host_version(&releases).as_deref(),
                expected,
                "tags: {tags:?}"
            );
        }
    }

    #[test]
    fn pick_latest_host_version_ignores_plugin_tags() {
        let releases = vec![
            rel("plugin-launcher-v1.8.0"),
            rel("plugin-alt-tab-v2.0.1"),
            rel("plugin-keyremap-v0.3.0"),
        ];
        assert_eq!(pick_latest_host_version(&releases), None);
    }

    #[test]
    fn pick_latest_host_version_picks_first_host_tag_when_mixed() {
        let releases = vec![
            rel("plugin-launcher-v1.8.0"),
            rel("qol-tray-v3.2.1"),
            rel("qol-tray-v3.1.0"),
            rel("plugin-alt-tab-v2.0.1"),
        ];
        assert_eq!(
            pick_latest_host_version(&releases).as_deref(),
            Some("3.2.1")
        );
    }

    #[test]
    fn pick_latest_host_version_returns_none_when_empty() {
        let releases: Vec<GitHubRelease> = vec![];
        assert_eq!(pick_latest_host_version(&releases), None);
    }

    #[test]
    fn pick_latest_host_version_rejects_collision_with_other_prefix() {
        let releases = vec![rel("qol-tray-doctor-v1.0.0")];
        assert_eq!(pick_latest_host_version(&releases), None);
    }

    #[test]
    fn is_newer_version_strict_ordering() {
        let cases = [
            ("1.2.4", "1.2.3", true),
            ("2.0.0", "1.99.99", true),
            ("1.2.3", "1.2.3", false),
            ("1.2.2", "1.2.3", false),
            ("0.6.0", "0.5.99", true),
        ];
        for (latest, current, expected) in cases {
            assert_eq!(
                is_newer_version(latest, current),
                expected,
                "latest={latest} current={current}"
            );
        }
    }

    #[test]
    fn host_tag_prefix_matches_release_workflow() {
        assert_eq!(HOST_TAG_PREFIX, "qol-tray");
    }

    #[test]
    fn github_repo_targets_monorepo() {
        assert_eq!(GITHUB_REPO, "qol-tools/qol");
    }
}
