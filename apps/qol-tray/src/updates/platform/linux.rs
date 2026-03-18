use anyhow::Result;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_stream::StreamExt;

use crate::daemon::{DaemonEvent, EventBus};

use super::super::{latest_version, GITHUB_REPO};

async fn download_asset(url: &str, dest: &Path, events: &EventBus) -> Result<()> {
    let request = crate::features::plugin_store::github::build_github_request(
        &reqwest::Client::new(),
        url,
        None,
    );
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
            events.send(DaemonEvent::UpdateProgress { percent });
            last_percent = percent;
        }
    }
    file.sync_all()?;
    Ok(())
}

fn asset_url(version: &str) -> String {
    format!(
        "https://github.com/{}/releases/download/v{}/qol-tray_{}-1_amd64.deb",
        GITHUB_REPO, version, version
    )
}

fn asset_filename(version: &str) -> String {
    format!("qol-tray_{}-1_amd64.deb", version)
}

fn resolve_update_url() -> Result<(String, PathBuf)> {
    #[cfg(feature = "dev")]
    if let Ok(url) = std::env::var("QOL_TRAY_DEV_UPDATE_URL") {
        let filename = url
            .split('/')
            .next_back()
            .unwrap_or("dev-update.deb")
            .to_string();
        return Ok((url, std::env::temp_dir().join(filename)));
    }

    let version = latest_version().ok_or_else(|| anyhow::anyhow!("No update version available"))?;
    Ok((
        asset_url(version),
        std::env::temp_dir().join(asset_filename(version)),
    ))
}

fn install_asset(path: &Path) -> Result<()> {
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

pub(super) async fn download_and_install(events: Arc<EventBus>) -> Result<()> {
    let (url, dest) = resolve_update_url()?;

    log::info!("Downloading update from {}", url);
    if let Err(e) = download_asset(&url, &dest, &events).await {
        let _ = std::fs::remove_file(&dest);
        return Err(e);
    }

    install_asset(&dest)?;
    events.send(DaemonEvent::UpdateComplete);
    // Give SSE time to deliver the event before exiting
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    log::info!("Update installed, restarting...");
    let binary = std::env::current_exe()?;
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let error = std::process::Command::new(&binary).args(&args).exec();
    anyhow::bail!("exec restart failed: {error}")
}
