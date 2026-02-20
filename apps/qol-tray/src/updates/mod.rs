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

    let client = reqwest::Client::new();
    let request = crate::features::plugin_store::github::build_github_request(&client, &url, None);
    let response = crate::features::plugin_store::github::send_checked(request).await?;

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
async fn download_asset(
    url: &str,
    dest: &std::path::Path,
    events: &crate::daemon::EventBus,
) -> Result<()> {
    use futures_util::StreamExt;
    use std::io::Write;

    let client = reqwest::Client::new();
    let request = crate::features::plugin_store::github::build_github_request(&client, url, None);
    let response = crate::features::plugin_store::github::send_checked(request).await?;

    let total = response.content_length();
    let mut stream = response.bytes_stream();
    let mut file = std::fs::File::create(dest)?;
    let mut downloaded: u64 = 0;
    let mut last_percent: u8 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;

        let percent = total
            .map(|t| ((downloaded * 100) / t).min(100) as u8)
            .unwrap_or(0);

        if percent != last_percent {
            events.send(crate::daemon::DaemonEvent::UpdateProgress { percent });
            last_percent = percent;
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn asset_url(version: &str) -> String {
    format!(
        "https://github.com/{}/releases/download/v{}/qol-tray_{}-1_amd64.deb",
        GITHUB_REPO, version, version
    )
}

#[cfg(target_os = "linux")]
fn asset_filename(version: &str) -> String {
    format!("qol-tray_{}-1_amd64.deb", version)
}

#[cfg(target_os = "linux")]
fn resolve_update_url() -> Result<(String, std::path::PathBuf)> {
    #[cfg(feature = "dev")]
    if let Ok(url) = std::env::var("QOL_TRAY_DEV_UPDATE_URL") {
        let filename = url.split('/').last().unwrap_or("dev-update.deb").to_string();
        return Ok((url, std::env::temp_dir().join(filename)));
    }

    let version = latest_version().ok_or_else(|| anyhow::anyhow!("No update version available"))?;
    Ok((asset_url(version), std::env::temp_dir().join(asset_filename(version))))
}

#[cfg(target_os = "linux")]
fn install_asset(path: &std::path::Path) -> Result<()> {
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
pub async fn download_and_install(events: std::sync::Arc<crate::daemon::EventBus>) -> Result<()> {
    let (url, dest) = resolve_update_url()?;

    log::info!("Downloading update from {}", url);
    download_asset(&url, &dest, &events).await?;
    events.send(crate::daemon::DaemonEvent::UpdateComplete);

    install_asset(&dest)?;

    log::info!("Update installed, restarting...");
    let restart_binary = std::env::current_exe()
        .map_err(|e| anyhow::anyhow!("Failed to resolve current executable for restart: {}", e))?;
    std::process::Command::new(&restart_binary)
        .spawn()
        .map_err(|e| anyhow::anyhow!("Failed to spawn {} for restart: {}", restart_binary.display(), e))?;
    std::process::exit(0);
}

#[cfg(target_os = "macos")]
pub async fn download_and_install(_events: std::sync::Arc<crate::daemon::EventBus>) -> Result<()> {
    let url = format!("https://github.com/{}/releases/latest", GITHUB_REPO);
    crate::paths::open_url(&url)?;
    Ok(())
}

#[cfg(target_os = "windows")]
pub async fn download_and_install(_events: std::sync::Arc<crate::daemon::EventBus>) -> Result<()> {
    let url = format!("https://github.com/{}/releases/latest", GITHUB_REPO);
    crate::paths::open_url(&url)?;
    Ok(())
}
