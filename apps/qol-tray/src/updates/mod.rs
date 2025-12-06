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
    use crate::version::Version;
    Version::parse(latest).is_newer_than(&Version::parse(current))
}

#[cfg(target_os = "linux")]
pub async fn download_and_install() -> Result<()> {
    let version = latest_version().ok_or_else(|| anyhow::anyhow!("No update version available"))?;
    let deb_path = download_deb(version).await?;
    install_deb(&deb_path)?;
    restart_with_cleanup();
}

#[cfg(target_os = "linux")]
async fn download_deb(version: &str) -> Result<std::path::PathBuf> {
    let url = format!(
        "https://github.com/{}/releases/download/v{}/qol-tray_{}-1_amd64.deb",
        GITHUB_REPO, version, version
    );
    let path = std::path::PathBuf::from(format!("/tmp/qol-tray_{}-1_amd64.deb", version));

    log::info!("Downloading update from {}", url);

    let client = reqwest::Client::builder().user_agent("qol-tray").build()?;
    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        anyhow::bail!("Failed to download update: {}", response.status());
    }

    let bytes = response.bytes().await?;
    std::fs::write(&path, &bytes)?;
    Ok(path)
}

#[cfg(target_os = "linux")]
fn install_deb(path: &std::path::Path) -> Result<()> {
    log::info!("Installing update...");

    let status = std::process::Command::new("pkexec")
        .args(["dpkg", "-i"])
        .arg(path)
        .status()?;

    if !status.success() {
        anyhow::bail!("Failed to install update");
    }

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[cfg(target_os = "linux")]
fn restart_with_cleanup() -> ! {
    log::info!("Update installed, stopping daemons...");

    let _ = std::process::Command::new("pkill")
        .args(["-f", "plugin-launcher/target"])
        .status();
    let _ = std::fs::remove_file("/tmp/qol-launcher.sock");

    log::info!("Restarting...");
    let _ = std::process::Command::new("qol-tray").spawn();
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
    fn is_newer_version_comparisons() {
        let cases = [
            // (latest, current, expected)
            ("2.0.0", "1.0.0", true),   // major bump
            ("1.1.0", "1.0.0", true),   // minor bump
            ("1.0.1", "1.0.0", true),   // patch bump
            ("1.0.0.1", "1.0.0", true), // extra segment
            ("1.0.0", "1.0.0", false),  // same version
            ("1.0.0", "2.0.0", false),  // older version
            ("1.0", "1.0.0", false),    // shorter version
        ];

        for (latest, current, expected) in cases {
            assert_eq!(
                is_newer_version(latest, current),
                expected,
                "is_newer_version({:?}, {:?})",
                latest,
                current
            );
        }
    }
}
