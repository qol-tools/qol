use anyhow::Result;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::sync::Arc;

use crate::daemon::{DaemonEvent, EventBus};

use super::super::{latest_version, GITHUB_REPO};
use super::common;
use super::InstallKind;

fn asset_url(version: &str) -> String {
    format!(
        "https://github.com/{}/releases/download/v{}/qol-tray-linux-{}.tar.gz",
        GITHUB_REPO,
        version,
        common::arch_suffix()
    )
}

fn asset_filename(version: &str) -> String {
    format!("qol-tray-linux-{}.tar.gz", version)
}

fn resolve_update_url() -> Result<(String, PathBuf)> {
    #[cfg(feature = "dev")]
    if let Ok(url) = std::env::var("QOL_TRAY_DEV_UPDATE_URL") {
        let filename = url
            .split('/')
            .next_back()
            .unwrap_or("dev-update.tar.gz")
            .to_string();
        return Ok((url, std::env::temp_dir().join(filename)));
    }

    let version = latest_version().ok_or_else(|| anyhow::anyhow!("No update version available"))?;
    Ok((
        asset_url(version),
        std::env::temp_dir().join(asset_filename(version)),
    ))
}

pub(super) async fn download_and_install(events: Arc<EventBus>) -> Result<()> {
    match InstallKind::detect() {
        InstallKind::SystemWide => {
            anyhow::bail!("This installation is managed by your system package manager")
        }
        InstallKind::Development => {
            anyhow::bail!("Self-update is disabled in development builds")
        }
        InstallKind::UserLocal => {}
    }

    let (url, dest) = resolve_update_url()?;

    log::info!("Downloading update from {}", url);
    if let Err(e) = common::download_asset(&url, &dest, &events).await {
        common::cleanup_archive(&dest);
        return Err(e);
    }

    let install_result = common::extract_tar_gz(&dest, "qol-tray")
        .and_then(|binary| common::atomic_replace(&binary, &std::env::current_exe()?));
    common::cleanup_archive(&dest);
    install_result?;

    events.send(DaemonEvent::UpdateComplete);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    log::info!("Update installed, restarting...");
    let binary = std::env::current_exe()?;
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let error = std::process::Command::new(&binary).args(&args).exec();
    anyhow::bail!("exec restart failed: {error}")
}
