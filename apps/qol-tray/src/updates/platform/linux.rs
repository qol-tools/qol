use anyhow::Result;
use std::os::unix::process::CommandExt;
use std::sync::Arc;

use crate::daemon::{DaemonEvent, EventBus};
use crate::features::plugin_store::release_integrity;

use super::super::{latest_version, GITHUB_REPO};
use super::common;
use super::InstallKind;

fn asset_name() -> String {
    format!("qol-tray-linux-{}.tar.gz", common::arch_suffix())
}

pub(super) async fn download_and_install(events: Arc<EventBus>) -> Result<()> {
    let dev_url = common::dev_update_url();
    let dev_override = dev_url.is_some();

    if !dev_override {
        match InstallKind::detect() {
            InstallKind::SystemWide => {
                log::warn!(
                    "Updating a system-wide installation — binary will be replaced in place"
                );
            }
            InstallKind::Development => {
                anyhow::bail!("Self-update is disabled in development builds")
            }
            InstallKind::UserLocal => {}
        }
    }

    let work_dir = tempfile::Builder::new()
        .prefix("qol-tray-update-")
        .tempdir()?;
    let dest = work_dir.path().join("update.tar.gz");
    let verified_asset = if dev_override {
        None
    } else {
        let version =
            latest_version().ok_or_else(|| anyhow::anyhow!("No update version available"))?;
        let release =
            release_integrity::fetch_release(GITHUB_REPO, &format!("qol-tray-v{version}")).await?;
        Some(release_integrity::verified_asset(&release, &asset_name())?)
    };
    let url = dev_url
        .or_else(|| {
            verified_asset
                .as_ref()
                .map(|asset| asset.browser_download_url.clone())
        })
        .ok_or_else(|| anyhow::anyhow!("No verified update asset available"))?;

    log::info!("Downloading update from {}", url);
    common::download_asset(&url, &dest, &events).await?;
    if let Some(asset) = &verified_asset {
        release_integrity::verify_file(asset, &dest)?;
    }

    let current_exe = std::env::current_exe()?;
    let install_result = common::extract_tar_gz(&dest, "qol-tray")
        .and_then(|binary| common::atomic_replace(&binary, &current_exe));
    install_result?;

    events.send(DaemonEvent::UpdateComplete);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    log::info!("Update installed, restarting...");
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let error = std::process::Command::new(&current_exe).args(&args).exec();
    anyhow::bail!("exec restart failed: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_release_asset_name_is_stable() {
        let arch = common::arch_suffix();
        assert_eq!(asset_name(), format!("qol-tray-linux-{arch}.tar.gz"));
    }
}
