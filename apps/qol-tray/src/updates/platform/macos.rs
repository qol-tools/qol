use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::daemon::{DaemonEvent, EventBus};

use super::super::{latest_version, GITHUB_REPO};
use super::common;
use super::InstallKind;

const APP_BUNDLE_NAME: &str = "QoL Tray.app";
const MACOS_RELEASE_ASSET: &str = "qol-tray-macos-universal.tar.gz";

fn asset_url(version: &str) -> String {
    format!(
        "https://github.com/{}/releases/download/v{}/{}",
        GITHUB_REPO, version, MACOS_RELEASE_ASSET
    )
}

fn asset_filename(_: &str) -> String {
    MACOS_RELEASE_ASSET.to_string()
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

fn ad_hoc_codesign_bundle(bundle_path: &Path) {
    let sign_status = std::process::Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(bundle_path)
        .output();
    match sign_status {
        Ok(out) if out.status.success() => {}
        Ok(out) => log::warn!(
            "codesign ad-hoc failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(e) => log::warn!("codesign not available: {e}"),
    }
}

fn find_app_bundle(exe_path: &Path) -> Option<PathBuf> {
    let mut current = exe_path;
    while let Some(parent) = current.parent() {
        if let Some(name) = parent.file_name() {
            if name.to_string_lossy().ends_with(".app") {
                return Some(parent.to_path_buf());
            }
        }
        current = parent;
    }
    None
}

fn replace_app_bundle(source_bundle: &Path, target_bundle: &Path) -> Result<()> {
    let parent = target_bundle
        .parent()
        .context("Current app bundle has no parent directory")?;
    let staging = staged_bundle_path(parent, target_bundle)?;
    let backup = backup_bundle_path(parent, target_bundle)?;

    cleanup_dir_if_exists(&staging);
    cleanup_dir_if_exists(&backup);
    copy_bundle_dir(source_bundle, &staging)?;

    if !target_bundle.exists() {
        fs::rename(&staging, target_bundle).with_context(|| {
            format!(
                "Failed to move staged bundle {} into place at {}",
                staging.display(),
                target_bundle.display()
            )
        })?;
        return Ok(());
    }

    fs::rename(target_bundle, &backup).with_context(|| {
        format!(
            "Failed to move current bundle {} to backup {}",
            target_bundle.display(),
            backup.display()
        )
    })?;

    let swap_result = fs::rename(&staging, target_bundle);
    if let Err(error) = swap_result {
        let rollback_result = fs::rename(&backup, target_bundle);
        if let Err(rollback_error) = rollback_result {
            anyhow::bail!(
                "Failed to replace app bundle: {}; rollback failed: {}",
                error,
                rollback_error
            );
        }
        anyhow::bail!("Failed to replace app bundle: {}", error);
    }

    cleanup_dir_if_exists(&backup);
    Ok(())
}

fn staged_bundle_path(parent: &Path, target_bundle: &Path) -> Result<PathBuf> {
    let name = target_bundle
        .file_name()
        .context("Current app bundle has no file name")?
        .to_string_lossy();
    Ok(parent.join(format!(".{}.updating.{}", name, unique_suffix())))
}

fn backup_bundle_path(parent: &Path, target_bundle: &Path) -> Result<PathBuf> {
    let name = target_bundle
        .file_name()
        .context("Current app bundle has no file name")?
        .to_string_lossy();
    Ok(parent.join(format!(".{}.backup.{}", name, unique_suffix())))
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}.{}", std::process::id(), nanos)
}

fn cleanup_dir_if_exists(path: &Path) {
    if !path.exists() {
        return;
    }
    let _ = fs::remove_dir_all(path);
}

fn copy_bundle_dir(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)
        .with_context(|| format!("Failed to create {}", destination.display()))?;

    for entry in walkdir::WalkDir::new(source) {
        let entry = entry?;
        let Ok(relative) = entry.path().strip_prefix(source) else {
            continue;
        };
        if relative.as_os_str().is_empty() {
            continue;
        }

        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)
                .with_context(|| format!("Failed to create {}", target.display()))?;
            continue;
        }

        fs::copy(entry.path(), &target).with_context(|| {
            format!(
                "Failed to copy {} to {}",
                entry.path().display(),
                target.display()
            )
        })?;
        let permissions = fs::metadata(entry.path())?.permissions();
        fs::set_permissions(&target, permissions)
            .with_context(|| format!("Failed to set permissions on {}", target.display()))?;
    }

    Ok(())
}

pub(super) async fn download_and_install(events: Arc<EventBus>) -> Result<()> {
    #[cfg(feature = "dev")]
    let dev_override = std::env::var("QOL_TRAY_DEV_UPDATE_URL").is_ok();
    #[cfg(not(feature = "dev"))]
    let dev_override = false;

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

    let (url, dest) = resolve_update_url()?;

    log::info!("Downloading update from {}", url);
    if let Err(e) = common::download_asset(&url, &dest, &events).await {
        common::cleanup_archive(&dest);
        return Err(e);
    }

    let current_exe = std::env::current_exe()?;
    let current_bundle =
        find_app_bundle(&current_exe).context("Current executable is not inside an app bundle")?;
    let install_result = super::common::extract_tar_gz_dir(&dest, APP_BUNDLE_NAME)
        .and_then(|bundle| replace_app_bundle(&bundle, &current_bundle));
    common::cleanup_archive(&dest);
    install_result?;

    if dev_override {
        ad_hoc_codesign_bundle(&current_bundle);
    }

    events.send(DaemonEvent::UpdateComplete);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    log::info!("Update installed, restarting...");
    exec_restart_on_main_thread()?;
    Ok(())
}

fn exec_restart_on_main_thread() -> Result<()> {
    use std::os::unix::process::CommandExt;

    let binary = std::env::current_exe()?;
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();

    extern "C" {
        static _dispatch_main_q: std::ffi::c_void;
        fn dispatch_async_f(
            queue: *const std::ffi::c_void,
            context: *mut std::ffi::c_void,
            work: extern "C" fn(*mut std::ffi::c_void),
        );
    }

    type ExecData = (PathBuf, Vec<std::ffi::OsString>);
    let data = Box::into_raw(Box::new((binary, args))) as *mut std::ffi::c_void;

    extern "C" fn do_exec(ctx: *mut std::ffi::c_void) {
        let (binary, args) = unsafe { *Box::from_raw(ctx as *mut ExecData) };
        eprintln!(
            "[qol-tray] exec'ing: {} (exists={}, args={:?})",
            binary.display(),
            binary.exists(),
            args
        );
        let error = std::process::Command::new(&binary).args(&args).exec();
        eprintln!("[qol-tray] update exec restart failed: {error}");
        std::process::exit(1);
    }

    unsafe {
        dispatch_async_f(&_dispatch_main_q, data, do_exec);
    }

    std::thread::sleep(std::time::Duration::from_secs(10));
    anyhow::bail!("exec did not happen within expected time")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_app_bundle(root: &Path, contents: &[(&str, &str)]) -> PathBuf {
        let app = root.join(APP_BUNDLE_NAME);
        for (relative, body) in contents {
            let path = app.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        app
    }

    #[test]
    fn find_app_bundle_from_exe_path() {
        let cases = [
            (
                "/a/Applications/Foo.app/Contents/MacOS/foo",
                Some("/a/Applications/Foo.app"),
            ),
            ("/a/.local/bin/foo", None),
            ("/usr/bin/foo", None),
        ];
        for (path, expected) in cases {
            assert_eq!(
                find_app_bundle(Path::new(path))
                    .as_deref()
                    .map(|p| p.to_str().unwrap()),
                expected,
                "path: {path}"
            );
        }
    }

    #[test]
    fn macos_release_asset_contract_is_stable() {
        let cases = [
            (
                "1.2.3",
                "qol-tray-macos-universal.tar.gz",
                "https://github.com/qol-tools/qol-tray/releases/download/v1.2.3/qol-tray-macos-universal.tar.gz",
            ),
            (
                "9.9.9-beta.1",
                "qol-tray-macos-universal.tar.gz",
                "https://github.com/qol-tools/qol-tray/releases/download/v9.9.9-beta.1/qol-tray-macos-universal.tar.gz",
            ),
        ];

        for (version, expected_name, expected_url) in cases {
            assert_eq!(asset_filename(version), expected_name, "version: {version}");
            assert_eq!(asset_url(version), expected_url, "version: {version}");
        }
    }

    #[test]
    fn replace_app_bundle_replaces_existing_bundle_contents() {
        let root = TempDir::new().unwrap();
        let source = create_app_bundle(
            root.path().join("source").as_path(),
            &[
                ("Contents/MacOS/qol-tray", "new-binary"),
                ("Contents/Info.plist", "new-plist"),
            ],
        );
        let target = create_app_bundle(
            root.path().join("target").as_path(),
            &[
                ("Contents/MacOS/qol-tray", "old-binary"),
                ("Contents/Resources/old.txt", "old-resource"),
            ],
        );

        replace_app_bundle(&source, &target).unwrap();

        assert_eq!(
            std::fs::read_to_string(target.join("Contents/MacOS/qol-tray")).unwrap(),
            "new-binary"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("Contents/Info.plist")).unwrap(),
            "new-plist"
        );
        assert!(!target.join("Contents/Resources/old.txt").exists());
    }

    #[test]
    fn replace_app_bundle_installs_when_target_is_missing() {
        let root = TempDir::new().unwrap();
        let source = create_app_bundle(
            root.path().join("source").as_path(),
            &[("Contents/MacOS/qol-tray", "new-binary")],
        );
        let target = root.path().join("Applications").join(APP_BUNDLE_NAME);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();

        replace_app_bundle(&source, &target).unwrap();

        assert_eq!(
            std::fs::read_to_string(target.join("Contents/MacOS/qol-tray")).unwrap(),
            "new-binary"
        );
    }
}
