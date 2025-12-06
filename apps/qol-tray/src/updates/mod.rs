use anyhow::Result;
use serde::Deserialize;
use std::sync::OnceLock;

static LATEST_VERSION: OnceLock<String> = OnceLock::new();

const GITHUB_REPO: &str = "qol-tools/qol-tray";
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

    let client = reqwest::Client::builder()
        .user_agent("qol-tray")
        .build()?;

    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        return Ok(false);
    }

    let release: GitHubRelease = response.json().await?;
    let latest = release.tag_name.trim_start_matches('v');

    if is_newer_version(latest, CURRENT_VERSION) {
        let _ = LATEST_VERSION.set(latest.to_string());
        log::info!(
            "Update available: {} -> {}",
            CURRENT_VERSION,
            latest
        );
        return Ok(true);
    }

    log::info!("No updates available (current: {})", CURRENT_VERSION);
    Ok(false)
}

fn is_newer_version(latest: &str, current: &str) -> bool {
    let parse_version = |v: &str| -> Vec<u32> {
        v.split('.')
            .filter_map(|s| s.parse().ok())
            .collect()
    };

    let latest_parts = parse_version(latest);
    let current_parts = parse_version(current);

    for (l, c) in latest_parts.iter().zip(current_parts.iter()) {
        if l > c {
            return true;
        }
        if l < c {
            return false;
        }
    }

    latest_parts.len() > current_parts.len()
}

pub fn open_releases_page() -> Result<()> {
    let url = format!("https://github.com/{}/releases/latest", GITHUB_REPO);

    #[cfg(target_os = "linux")]
    std::process::Command::new("xdg-open").arg(&url).spawn()?;

    #[cfg(target_os = "macos")]
    std::process::Command::new("open").arg(&url).spawn()?;

    #[cfg(target_os = "windows")]
    std::process::Command::new("cmd")
        .args(["/C", "start", &url])
        .spawn()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_newer_version_returns_true_for_major_bump() {
        // Arrange
        let latest = "2.0.0";
        let current = "1.0.0";

        // Act
        let result = is_newer_version(latest, current);

        // Assert
        assert!(result);
    }

    #[test]
    fn is_newer_version_returns_true_for_minor_bump() {
        // Arrange
        let latest = "1.1.0";
        let current = "1.0.0";

        // Act
        let result = is_newer_version(latest, current);

        // Assert
        assert!(result);
    }

    #[test]
    fn is_newer_version_returns_true_for_patch_bump() {
        // Arrange
        let latest = "1.0.1";
        let current = "1.0.0";

        // Act
        let result = is_newer_version(latest, current);

        // Assert
        assert!(result);
    }

    #[test]
    fn is_newer_version_returns_false_for_same_version() {
        // Arrange
        let latest = "1.0.0";
        let current = "1.0.0";

        // Act
        let result = is_newer_version(latest, current);

        // Assert
        assert!(!result);
    }

    #[test]
    fn is_newer_version_returns_false_for_older_version() {
        // Arrange
        let latest = "1.0.0";
        let current = "2.0.0";

        // Act
        let result = is_newer_version(latest, current);

        // Assert
        assert!(!result);
    }

    #[test]
    fn is_newer_version_handles_different_lengths() {
        // Arrange & Act & Assert
        assert!(is_newer_version("1.0.0.1", "1.0.0"));
        assert!(!is_newer_version("1.0", "1.0.0"));
    }
}
