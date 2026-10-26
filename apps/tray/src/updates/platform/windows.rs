use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::daemon::{DaemonEvent, EventBus};
use crate::plugins::PluginManager;

use super::super::{verify_host_update, verify_release_binary, FoundUpdate};
use super::download::{self, HostUpdate};
use super::windows_install_kind::install_kind_for;
use super::InstallKind;

const RETIRED_SUFFIX: &str = "old";
const STAGED_SUFFIX: &str = "new";

pub(super) fn recover_interrupted_update() {
    let Ok(target) =
        crate::paths::tray_installed_binary(qol_apps::tray_install::InstallProfile::Production)
    else {
        return;
    };
    match restore_retired_binary(&target) {
        Ok(true) => log::warn!(
            "restored {} from the copy an interrupted update left aside",
            target.display()
        ),
        Ok(false) => {}
        Err(error) => log::error!(
            "failed to restore {} after an interrupted update: {error:#}",
            target.display()
        ),
    }
}

fn restore_retired_binary(target: &Path) -> Result<bool> {
    let retired = sibling(target, RETIRED_SUFFIX);
    if target.exists() || !retired.is_file() {
        return Ok(false);
    }
    restore(&retired, target, |from: &Path, to: &Path| {
        std::fs::rename(from, to)
    })?;
    Ok(true)
}

fn restore(
    retired: &Path,
    target: &Path,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    rename(retired, target).or_else(|_| std::fs::copy(retired, target).map(drop))
}

pub(super) fn detect_install_kind() -> InstallKind {
    let Ok(executable) = std::env::current_exe() else {
        return InstallKind::SystemWide;
    };
    let install_dir =
        crate::paths::tray_install_dir(qol_apps::tray_install::InstallProfile::Production).ok();
    install_kind_for(&executable, install_dir.as_deref())
}

fn asset_name() -> String {
    format!("qol-tray-windows-{}.exe", std::env::consts::ARCH)
}

fn installer_asset_name() -> String {
    format!(
        "{}-windows-{}.exe",
        qol_conventions::artifact::TRAY_INSTALLER_BINARY_NAME,
        std::env::consts::ARCH
    )
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
        let rollback = restore(&retired, target, &mut rename);
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
    update: Option<FoundUpdate>,
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
    let update = if dev_override {
        None
    } else {
        Some(download::resolve_update(update).await?)
    };
    download::download_update_asset(update.as_ref(), dev_url, &asset_name(), &dest, &events)
        .await?;
    verify_host_update(
        &dest,
        update.as_ref().map(HostUpdate::version),
        qol_artifact::ArtifactExpectation::with_exact_target,
    )?;

    let installer = match &update {
        Some(update) => download_installer(update, work_dir.path()).await,
        None => None,
    };

    let current_exe = std::env::current_exe()?;
    replace_running_binary(&dest, &current_exe)?;
    if let Err(error) = crate::installer::platform::windows::refresh_after_update(
        &current_exe,
        installer.as_deref(),
    ) {
        log::warn!("Refreshing the installed shortcut and uninstaller failed: {error:#}");
    }
    if let Err(error) = work_dir.close() {
        log::warn!("Failed to remove the update download folder: {error}");
    }

    events.send(DaemonEvent::UpdateComplete);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    log::info!("Update installed, restarting...");
    crate::window_reopen::capture_before_restart();
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    match crate::relaunch::spawn_successor(&current_exe, &args) {
        Ok(pid) => {
            stop_plugins(plugin_manager).await;
            crate::relaunch::exit_to_successor(pid)
        }
        Err(error) => {
            crate::window_reopen::discard_reopen_list();
            anyhow::bail!("restart after update failed: {error}")
        }
    }
}

async fn download_installer(update: &HostUpdate, work_dir: &Path) -> Option<PathBuf> {
    let name = installer_asset_name();
    let asset = match update.asset(&name) {
        Ok(asset) => asset,
        Err(error) => {
            log::warn!("Keeping the installed uninstaller: {error:#}");
            return None;
        }
    };
    let dest = work_dir.join(&name);
    let downloaded = async {
        asset.download(&dest, |_| {}).await?;
        verify_release_binary(
            &dest,
            qol_conventions::artifact::TRAY_INSTALLER_BINARY_NAME,
            Some(update.version()),
            qol_artifact::ArtifactExpectation::with_exact_target,
        )
    };
    match downloaded.await {
        Ok(()) => Some(dest),
        Err(error) => {
            log::warn!("Keeping the installed uninstaller: {name} was not verified: {error:#}");
            None
        }
    }
}

async fn stop_plugins(plugin_manager: Arc<Mutex<PluginManager>>) {
    let stopping = tokio::task::spawn_blocking(move || {
        let mut manager = plugin_manager
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        manager.hand_off_running_daemons();
        manager.shutdown();
    });
    match tokio::time::timeout(crate::relaunch::PREDECESSOR_CLEANUP_BUDGET, stopping).await {
        Ok(Err(error)) => {
            log::error!("Stopping plugins before the update restart failed: {error}");
        }
        Ok(Ok(())) => {}
        Err(_) => log::warn!(
            "Stopping plugins before the update restart exceeded {:?}; exiting anyway",
            crate::relaunch::PREDECESSOR_CLEANUP_BUDGET
        ),
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

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Rollback {
        Renamed,
        Copied,
        Lost,
    }

    #[test]
    fn a_failed_swap_restores_the_previous_binary() {
        for rollback in [Rollback::Renamed, Rollback::Copied, Rollback::Lost] {
            let dir = tempfile::tempdir().unwrap();
            let source = dir.path().join("download.exe");
            let target = dir.path().join("qol-tray.exe");
            let staged = sibling(&target, STAGED_SUFFIX);
            let retired = sibling(&target, RETIRED_SUFFIX);
            std::fs::write(&source, b"new").unwrap();
            std::fs::write(&target, b"old").unwrap();

            let error = replace_binary_with(&source, &target, |from, to| {
                if from == staged {
                    return Err(std::io::Error::other("refused"));
                }
                if from == retired && rollback != Rollback::Renamed {
                    if rollback == Rollback::Lost {
                        std::fs::remove_file(from)?;
                    }
                    return Err(std::io::Error::other("held"));
                }
                std::fs::rename(from, to)
            })
            .unwrap_err();

            let message = format!("{error:#}");
            assert!(
                message.contains("Failed to replace"),
                "{rollback:?} message: {message}"
            );
            assert_eq!(
                message.contains("restoring the previous binary also failed"),
                rollback == Rollback::Lost,
                "{rollback:?} message: {message}"
            );
            assert!(!staged.exists(), "{rollback:?}");
            let target_bytes = std::fs::read(&target).ok();
            let retired_bytes = std::fs::read(&retired).ok();
            let expected: (Option<&[u8]>, Option<&[u8]>) = match rollback {
                Rollback::Renamed => (Some(&b"old"[..]), None),
                Rollback::Copied => (Some(&b"old"[..]), Some(&b"old"[..])),
                Rollback::Lost => (None, None),
            };
            assert_eq!(
                (target_bytes.as_deref(), retired_bytes.as_deref()),
                expected,
                "{rollback:?}"
            );
        }
    }

    #[test]
    fn a_missing_binary_is_restored_from_the_copy_an_update_left_aside() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("qol-tray.exe");
        let retired = sibling(&target, RETIRED_SUFFIX);
        std::fs::write(&retired, b"old").unwrap();

        assert!(restore_retired_binary(&target).unwrap());

        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert!(!retired.exists());
    }

    #[test]
    fn a_present_binary_is_left_alone_by_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("qol-tray.exe");
        let retired = sibling(&target, RETIRED_SUFFIX);

        assert!(!restore_retired_binary(&target).unwrap());

        std::fs::write(&target, b"new").unwrap();
        std::fs::write(&retired, b"old").unwrap();
        assert!(!restore_retired_binary(&target).unwrap());
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(std::fs::read(&retired).unwrap(), b"old");
    }

    #[test]
    fn a_held_previous_binary_is_restored_by_copying() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("qol-tray.exe");
        let retired = sibling(&target, RETIRED_SUFFIX);
        std::fs::write(&retired, b"old").unwrap();

        restore(&retired, &target, |_, _| Err(std::io::Error::other("held"))).unwrap();

        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert_eq!(std::fs::read(&retired).unwrap(), b"old");
    }

    #[test]
    fn the_installer_asset_follows_the_release_naming() {
        assert_eq!(
            installer_asset_name(),
            "qol-tray-install-windows-x86_64.exe"
        );
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
