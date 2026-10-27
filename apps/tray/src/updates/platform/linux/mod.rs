use anyhow::{Context, Result};
use std::os::unix::process::CommandExt;
use std::sync::{Arc, Mutex};

use crate::daemon::{DaemonEvent, EventBus};
use crate::plugins::PluginManager;

use super::super::FoundUpdate;
use super::download::HostUpdate;
use super::InstallKind;
use super::{download, unix};

mod installation;

pub(super) fn recover_interrupted_update() {}

pub(super) fn detect_install_kind() -> InstallKind {
    let executable = std::env::current_exe()
        .and_then(|path| std::fs::canonicalize(&path).or(Ok(path)))
        .ok();
    let executable = executable
        .as_deref()
        .and_then(|path| path.to_str())
        .unwrap_or_default();
    let home = crate::paths::user_home_dir()
        .ok()
        .and_then(|path| path.to_str().map(String::from));
    InstallKind::for_path(executable, home.as_deref(), false)
}

fn asset_name() -> String {
    format!("qol-tray-linux-{}.tar.gz", arch_suffix())
}

fn arch_suffix() -> &'static str {
    std::env::consts::ARCH
}

fn tar_update_blocked(
    kind: InstallKind,
    ownership: Result<
        qol_host_fixes::policy::managed::DpkgOwnership,
        qol_host_fixes::policy::managed::CarrierError,
    >,
) -> Result<bool> {
    Ok(match kind {
        InstallKind::Development => true,
        InstallKind::UserLocal => false,
        InstallKind::SystemWide => match ownership {
            Ok(qol_host_fixes::policy::managed::DpkgOwnership::PackageOwnsPath) => true,
            Ok(
                qol_host_fixes::policy::managed::DpkgOwnership::PathNotPackageOwned
                | qol_host_fixes::policy::managed::DpkgOwnership::NoDpkg,
            ) => false,
            Err(error) => {
                return Err(anyhow::anyhow!(
                    "cannot determine the qol-tray install ownership: {error}"
                ));
            }
        },
    })
}

pub(super) async fn download_and_install(
    events: Arc<EventBus>,
    _plugin_manager: Arc<Mutex<PluginManager>>,
    update: Option<FoundUpdate>,
) -> Result<()> {
    let install_kind = InstallKind::detect();
    log::info!("Install kind: {install_kind:?}");
    let dev_url = download::dev_update_url();
    let dev_override = dev_url.is_some();

    if !dev_override {
        match install_kind {
            InstallKind::SystemWide => {
                let current = std::env::current_exe().context("current executable")?;
                let ownership = qol_host_fixes::policy::managed::dpkg_ownership(
                    &current.to_string_lossy(),
                    qol_conventions::artifact::TRAY_PACKAGE_NAME,
                );
                if tar_update_blocked(install_kind, ownership)? {
                    anyhow::bail!(
                        "qol-tray is installed system-wide from a Debian package; the host-only \
                         tarball cannot update it (the package's maintainer scripts own the update \
                         path). Update with the package manager: apt-get update && apt-get install \
                         --only-upgrade qol-tray, or reinstall the new package with: apt-get install \
                         ./qol-tray_<version>_<arch>.deb"
                    )
                }
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
    let update = if dev_override {
        None
    } else {
        Some(download::resolve_update(update).await?)
    };
    let expected_version = update.as_ref().map(HostUpdate::version);
    download::download_update_asset(update.as_ref(), dev_url, &asset_name(), &dest, &events)
        .await?;

    let current_exe = std::env::current_exe()?;
    let binary = unix::extract_tar_gz_entry(&dest, "qol-tray", false)?;
    let target = current_exe.clone();
    let expected_version = expected_version.map(str::to_owned);
    tokio::task::spawn_blocking(move || {
        installation::install(&binary, &target, expected_version.as_deref())
    })
    .await
    .context("Update installation task failed")??;

    events.send(DaemonEvent::UpdateComplete);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    log::info!("Update installed, restarting...");
    crate::window_reopen::capture_before_restart();
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let error = std::process::Command::new(&current_exe).args(&args).exec();
    crate::window_reopen::discard_reopen_list();
    anyhow::bail!("exec restart failed: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::updates::platform::install_kind::InstallKind;

    fn deb_owned() -> Result<
        qol_host_fixes::policy::managed::DpkgOwnership,
        qol_host_fixes::policy::managed::CarrierError,
    > {
        Ok(qol_host_fixes::policy::managed::DpkgOwnership::PackageOwnsPath)
    }

    fn foreign_owned() -> Result<
        qol_host_fixes::policy::managed::DpkgOwnership,
        qol_host_fixes::policy::managed::CarrierError,
    > {
        Ok(qol_host_fixes::policy::managed::DpkgOwnership::PathNotPackageOwned)
    }

    fn no_dpkg() -> Result<
        qol_host_fixes::policy::managed::DpkgOwnership,
        qol_host_fixes::policy::managed::CarrierError,
    > {
        Ok(qol_host_fixes::policy::managed::DpkgOwnership::NoDpkg)
    }

    fn indeterminate() -> Result<
        qol_host_fixes::policy::managed::DpkgOwnership,
        qol_host_fixes::policy::managed::CarrierError,
    > {
        Err(
            qol_host_fixes::policy::managed::CarrierError::PackageQueryFailed {
                detail: "injected".to_string(),
            },
        )
    }

    #[test]
    fn tar_update_blocking_is_a_pure_decision_matrix() {
        assert!(tar_update_blocked(InstallKind::Development, deb_owned()).unwrap());
        assert!(tar_update_blocked(InstallKind::Development, no_dpkg()).unwrap());
        assert!(!tar_update_blocked(InstallKind::UserLocal, deb_owned()).unwrap());
        assert!(!tar_update_blocked(InstallKind::UserLocal, indeterminate()).unwrap());
        assert!(tar_update_blocked(InstallKind::SystemWide, deb_owned()).unwrap());
        assert!(!tar_update_blocked(InstallKind::SystemWide, foreign_owned()).unwrap());
        assert!(!tar_update_blocked(InstallKind::SystemWide, no_dpkg()).unwrap());
        assert!(tar_update_blocked(InstallKind::SystemWide, indeterminate()).is_err());
    }

    #[test]
    fn linux_release_asset_name_is_stable() {
        let arch = arch_suffix();
        assert_eq!(asset_name(), format!("qol-tray-linux-{arch}.tar.gz"));
    }
}
