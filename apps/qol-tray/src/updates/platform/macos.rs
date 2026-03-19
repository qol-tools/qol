use anyhow::Result;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::daemon::{DaemonEvent, EventBus};

use super::super::{latest_version, GITHUB_REPO};
use super::common;
use super::InstallKind;

fn asset_url(version: &str) -> String {
    format!(
        "https://github.com/{}/releases/download/v{}/qol-tray-macos-{}.tar.gz",
        GITHUB_REPO,
        version,
        common::arch_suffix()
    )
}

fn asset_filename(version: &str) -> String {
    format!("qol-tray-macos-{}.tar.gz", version)
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

fn codesign_bundle(exe_path: &Path) {
    let bundle_path = find_app_bundle(exe_path);
    let target = bundle_path.as_deref().unwrap_or(exe_path);
    let sign_status = std::process::Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(target)
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

    let current_exe = std::env::current_exe()?;
    let install_result = common::extract_tar_gz(&dest, "qol-tray")
        .and_then(|binary| common::atomic_replace(&binary, &current_exe));
    common::cleanup_archive(&dest);
    install_result?;

    codesign_bundle(&current_exe);

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
}
