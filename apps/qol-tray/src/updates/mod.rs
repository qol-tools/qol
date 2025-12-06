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

#[cfg(target_os = "linux")]
pub async fn download_and_install() -> Result<()> {
    let version = latest_version().ok_or_else(|| anyhow::anyhow!("No update version available"))?;

    let deb_url = format!(
        "https://github.com/{}/releases/download/v{}/qol-tray_{}_amd64.deb",
        GITHUB_REPO, version, version
    );

    let tmp_path = format!("/tmp/qol-tray_{}_amd64.deb", version);

    log::info!("Downloading update from {}", deb_url);

    let client = reqwest::Client::builder()
        .user_agent("qol-tray")
        .build()?;

    let response = client.get(&deb_url).send().await?;
    if !response.status().is_success() {
        anyhow::bail!("Failed to download update: {}", response.status());
    }

    let bytes = response.bytes().await?;
    std::fs::write(&tmp_path, &bytes)?;

    log::info!("Installing update...");

    let status = std::process::Command::new("pkexec")
        .args(["dpkg", "-i", &tmp_path])
        .status()?;

    if !status.success() {
        anyhow::bail!("Failed to install update");
    }

    let _ = std::fs::remove_file(&tmp_path);

    log::info!("Update installed successfully, restarting...");

    std::process::Command::new("qol-tray").spawn()?;
    std::process::exit(0);
}

#[cfg(target_os = "macos")]
pub async fn download_and_install() -> Result<()> {
    let url = format!("https://github.com/{}/releases/latest", GITHUB_REPO);
    std::process::Command::new("open").arg(&url).spawn()?;
    Ok(())
}

#[cfg(target_os = "windows")]
pub async fn download_and_install() -> Result<()> {
    let url = format!("https://github.com/{}/releases/latest", GITHUB_REPO);
    std::process::Command::new("cmd").args(["/C", "start", &url]).spawn()?;
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
