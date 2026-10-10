use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::daemon::{DaemonEvent, EventBus};
use crate::features::plugin_store::release_integrity;
use crate::plugins::PluginManager;

use super::super::{latest_version, verify_host_update, GITHUB_REPO};
use super::download;
use super::windows_install_kind::install_kind_for;
use super::InstallKind;

const RETIRED_SUFFIX: &str = "old";
const STAGED_SUFFIX: &str = "new";

pub(super) fn detect_install_kind() -> InstallKind {
    let Ok(executable) = std::env::current_exe() else {
        return InstallKind::SystemWide;
    };
    let install_dir = qol_apps::tray_install::install_dir().ok();
    install_kind_for(&executable, install_dir.as_deref())
}

fn asset_name() -> String {
    format!("qol-tray-windows-{}.exe", std::env::consts::ARCH)
}

fn sibling(target: &Path, suffix: &str) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(suffix);
    target.with_file_name(name)
}

fn replace_running_binary(source: &Path, target: &Path) -> Result<()> {
    replace_binary_with(source, target, |from: &Path, to: &Path| {
        std::fs::rename(from, to)
    })
}

fn replace_binary_with(
    source: &Path,
    target: &Path,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> Result<()> {
    let staged = sibling(target, STAGED_SUFFIX);
    let retired = sibling(target, RETIRED_SUFFIX);
    let _ = std::fs::remove_file(&staged);
    let _ = std::fs::remove_file(&retired);
    std::fs::copy(source, &staged).with_context(|| {
        format!(
            "Failed to stage {} to {}",
            source.display(),
            staged.display()
        )
    })?;
    if target.exists() {
        if let Err(error) = rename(target, &retired) {
            let _ = std::fs::remove_file(&staged);
            return Err(error)
                .with_context(|| format!("Failed to move aside {}", target.display()));
        }
    }
    if let Err(error) = rename(&staged, target) {
        let rollback = rename(&retired, target);
        let _ = std::fs::remove_file(&staged);
        return match rollback {
            Ok(()) => Err(error).with_context(|| format!("Failed to replace {}", target.display())),
            Err(rollback_error) => Err(error).with_context(|| {
                format!(
                    "Failed to replace {}; restoring the previous binary also failed: {rollback_error}",
                    target.display()
                )
            }),
        };
    }
    Ok(())
}

pub(super) async fn download_and_install(
    events: Arc<EventBus>,
    plugin_manager: Arc<Mutex<PluginManager>>,
) -> Result<()> {
    let install_kind = InstallKind::detect();
    log::info!("Install kind: {install_kind:?}");
    let dev_url = download::dev_update_url();
    let dev_override = dev_url.is_some();

    if !dev_override {
        match install_kind {
            InstallKind::SystemWide => {
                log::warn!(
                    "Updating a qol-tray outside the per-user install directory; the binary is replaced in place"
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
    let dest = work_dir.path().join(asset_name());
    let expected_version = if dev_override {
        None
    } else {
        Some(latest_version().ok_or_else(|| anyhow::anyhow!("No update version available"))?)
    };
    let verified_asset = if let Some(version) = expected_version.as_deref() {
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
    download::download_asset(&url, &dest, &events).await?;
    if let Some(asset) = &verified_asset {
        release_integrity::verify_file(asset, &dest)?;
    }
    verify_host_update(
        &dest,
        expected_version.as_deref(),
        qol_artifact::ArtifactExpectation::with_exact_target,
    )?;

    let current_exe = std::env::current_exe()?;
    replace_running_binary(&dest, &current_exe)?;

    events.send(DaemonEvent::UpdateComplete);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    log::info!("Update installed, restarting...");
    crate::window_reopen::capture_before_restart();
    stop_plugins(plugin_manager).await;
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let error = crate::relaunch::spawn_successor_and_exit(&current_exe, &args);
    crate::window_reopen::discard_reopen_list();
    anyhow::bail!("restart after update failed: {error}")
}

async fn stop_plugins(plugin_manager: Arc<Mutex<PluginManager>>) {
    let stopped = tokio::task::spawn_blocking(move || {
        plugin_manager
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .shutdown();
    })
    .await;
    if let Err(error) = stopped {
        log::error!("Stopping plugins before the update restart failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_release_asset_matches_the_plugin_naming() {
        assert_eq!(asset_name(), "qol-tray-windows-x86_64.exe");
    }

    #[test]
    fn replacing_swaps_the_binary_and_keeps_the_previous_one_aside() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("download.exe");
        let target = dir.path().join("qol-tray.exe");
        std::fs::write(&source, b"new").unwrap();
        std::fs::write(&target, b"old").unwrap();
        std::fs::write(sibling(&target, RETIRED_SUFFIX), b"older").unwrap();

        replace_running_binary(&source, &target).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(
            std::fs::read(sibling(&target, RETIRED_SUFFIX)).unwrap(),
            b"old"
        );
        assert!(!sibling(&target, STAGED_SUFFIX).exists());
    }

    #[test]
    fn a_failed_swap_restores_the_previous_binary() {
        for restore_fails in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("download.exe");
            let target = dir.path().join("qol-tray.exe");
            let staged = sibling(&target, STAGED_SUFFIX);
            let retired = sibling(&target, RETIRED_SUFFIX);
            std::fs::write(&source, b"new").unwrap();
            std::fs::write(&target, b"old").unwrap();

            let error = replace_binary_with(&source, &target, |from, to| {
                let refused = from == staged || (restore_fails && from == retired);
                if refused {
                    return Err(std::io::Error::other("refused"));
                }
                std::fs::rename(from, to)
            })
            .unwrap_err();

            let message = format!("{error:#}");
            assert!(
                message.contains("Failed to replace"),
                "restore_fails: {restore_fails} message: {message}"
            );
            assert_eq!(
                message.contains("restoring the previous binary also failed"),
                restore_fails,
                "restore_fails: {restore_fails} message: {message}"
            );
            assert!(!staged.exists(), "restore_fails: {restore_fails}");
            let (restored, aside) = if restore_fails {
                (&retired, &target)
            } else {
                (&target, &retired)
            };
            assert_eq!(
                std::fs::read(restored).unwrap(),
                b"old",
                "restore_fails: {restore_fails}"
            );
            assert!(!aside.exists(), "restore_fails: {restore_fails}");
        }
    }

    #[test]
    fn replacing_a_running_executable_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("running.exe");
        let system_root = std::env::var_os("SystemRoot").expect("SystemRoot");
        let ping = Path::new(&system_root).join("System32").join("PING.EXE");
        std::fs::copy(ping, &target).unwrap();
        let mut child = std::process::Command::new(&target)
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let source = dir.path().join("download.exe");
        std::fs::write(&source, b"new").unwrap();

        let replaced = replace_running_binary(&source, &target);
        let _ = child.kill();
        let _ = child.wait();

        replaced.unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
    }
}
