use anyhow::Result;
use std::io::Write;
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

fn arch_suffix() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    }
}

fn asset_url(version: &str) -> String {
    format!(
        "https://github.com/{}/releases/download/v{}/qol-tray-macos-{}.tar.gz",
        GITHUB_REPO,
        version,
        arch_suffix()
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

fn extract_binary(archive: &Path) -> Result<PathBuf> {
    let tar_gz = std::fs::File::open(archive)?;
    let tar = flate2::read::GzDecoder::new(tar_gz);
    let mut archive_reader = tar::Archive::new(tar);

    let extract_dir = archive.with_extension("extracted");
    if extract_dir.exists() {
        let _ = std::fs::remove_dir_all(&extract_dir);
    }
    std::fs::create_dir_all(&extract_dir)?;
    archive_reader.unpack(&extract_dir)?;

    // Binary is at <extract_dir>/<bundle_name>/qol-tray
    let binary = find_binary(&extract_dir, "qol-tray")?;
    Ok(binary)
}

fn find_binary(dir: &Path, name: &str) -> Result<PathBuf> {
    for entry in walkdir::WalkDir::new(dir).max_depth(2) {
        let entry = entry?;
        if entry.file_type().is_file() && entry.file_name().to_string_lossy() == name {
            return Ok(entry.into_path());
        }
    }
    anyhow::bail!("Binary '{}' not found in extracted archive", name)
}

fn install_binary(source: &Path) -> Result<()> {
    log::info!("Installing update from {}", source.display());
    let current_exe = std::env::current_exe()?;
    let staged = current_exe.with_extension("new");

    if staged.exists() {
        let _ = std::fs::remove_file(&staged);
    }

    std::fs::copy(source, &staged)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&staged)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&staged, perms)?;
    }

    std::fs::rename(&staged, &current_exe)?;

    // Re-sign with ad-hoc signature — patched or downloaded binaries
    // invalidate the original code signature, and macOS kills unsigned
    // executables with SIGKILL on exec.
    let sign_status = std::process::Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(&current_exe)
        .output();
    match sign_status {
        Ok(out) if out.status.success() => {}
        Ok(out) => log::warn!(
            "codesign ad-hoc failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(e) => log::warn!("codesign not available: {e}"),
    }

    Ok(())
}

fn cleanup(archive: &Path) {
    let _ = std::fs::remove_file(archive);
    let extract_dir = archive.with_extension("extracted");
    let _ = std::fs::remove_dir_all(&extract_dir);
}

pub(super) async fn download_and_install(events: Arc<EventBus>) -> Result<()> {
    let (url, dest) = resolve_update_url()?;

    log::info!("Downloading update from {}", url);
    if let Err(e) = download_asset(&url, &dest, &events).await {
        cleanup(&dest);
        return Err(e);
    }

    let install_result = extract_binary(&dest).and_then(|binary| install_binary(&binary));
    cleanup(&dest);
    install_result?;

    events.send(DaemonEvent::UpdateComplete);
    // Give SSE time to deliver the event before exiting
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

    // Park — the main thread will exec and replace the process.
    std::thread::sleep(std::time::Duration::from_secs(10));
    anyhow::bail!("exec did not happen within expected time")
}
