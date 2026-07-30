use anyhow::{Context, Result};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::sync::Arc;

use crate::daemon::{DaemonEvent, EventBus};
use crate::features::plugin_store::release_integrity;

use super::super::{latest_version, verify_host_update, GITHUB_REPO};
use super::unix;
use super::InstallKind;

pub(super) fn detect_install_kind() -> InstallKind {
    let executable = std::env::current_exe()
        .and_then(|path| std::fs::canonicalize(&path).or(Ok(path)))
        .ok();
    let executable = executable
        .as_deref()
        .and_then(|path| path.to_str())
        .unwrap_or_default();
    let home = dirs::home_dir().and_then(|path| path.to_str().map(String::from));
    InstallKind::for_path(executable, home.as_deref(), false)
}

fn asset_name() -> String {
    format!("qol-tray-linux-{}.tar.gz", arch_suffix())
}

fn arch_suffix() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => other,
    }
}

fn atomic_replace(source: &Path, target: &Path) -> Result<()> {
    let staged = target.with_extension("new");
    let result = atomic_replace_inner(source, target, &staged);
    if result.is_err() && staged.exists() {
        let _ = std::fs::remove_file(&staged);
    }
    result
}

fn atomic_replace_inner(source: &Path, target: &Path, staged: &Path) -> Result<()> {
    if staged.exists() {
        let _ = std::fs::remove_file(staged);
    }
    std::fs::copy(source, staged).with_context(|| {
        format!(
            "Failed to stage {} to {}",
            source.display(),
            staged.display()
        )
    })?;

    let mut perms = std::fs::metadata(staged)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(staged, perms)?;

    std::fs::rename(staged, target)
        .with_context(|| format!("Failed to replace {}", target.display()))?;
    Ok(())
}

pub(super) async fn download_and_install(events: Arc<EventBus>) -> Result<()> {
    let install_kind = InstallKind::detect();
    log::info!("Install kind: {install_kind:?}");
    let dev_url = unix::dev_update_url();
    let dev_override = dev_url.is_some();

    if !dev_override {
        match install_kind {
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
    let expected_version = if dev_override {
        None
    } else {
        Some(latest_version().ok_or_else(|| anyhow::anyhow!("No update version available"))?)
    };
    let verified_asset = if let Some(version) = expected_version {
        let release =
            release_integrity::fetch_release(GITHUB_REPO, &format!("qol-tray-v{version}")).await?;
        Some(release_integrity::verified_asset(&release, &asset_name())?)
    } else {
        None
    };
    let url = dev_url
        .or_else(|| {
            verified_asset
                .as_ref()
                .map(|asset| asset.browser_download_url.clone())
        })
        .ok_or_else(|| anyhow::anyhow!("No verified update asset available"))?;

    log::info!("Downloading update from {}", url);
    unix::download_asset(&url, &dest, &events).await?;
    if let Some(asset) = &verified_asset {
        release_integrity::verify_file(asset, &dest)?;
    }

    let current_exe = std::env::current_exe()?;
    let install_result = unix::extract_tar_gz_entry(&dest, "qol-tray", false).and_then(|binary| {
        verify_host_update(
            &binary,
            expected_version,
            qol_artifact::ArtifactExpectation::with_exact_target,
        )?;
        atomic_replace(&binary, &current_exe)
    });
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
        let arch = arch_suffix();
        assert_eq!(asset_name(), format!("qol-tray-linux-{arch}.tar.gz"));
    }

    #[test]
    fn atomic_replace_swaps_file() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let target = dir.path().join("target");
        std::fs::write(&source, b"new content").unwrap();
        std::fs::write(&target, b"old content").unwrap();
        atomic_replace(&source, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new content");
        assert!(!dir.path().join("target.new").exists());
    }

    #[test]
    fn atomic_replace_creates_target_if_missing() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let target = dir.path().join("target");
        std::fs::write(&source, b"content").unwrap();
        atomic_replace(&source, &target).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"content");
    }
}
