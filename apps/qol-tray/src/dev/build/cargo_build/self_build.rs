mod artifacts;

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU32, Ordering};

use super::super::types::BuildResult;
use super::{spawn_piped, CargoChild};

static LAST_ARTIFACT_COUNT: AtomicU32 = AtomicU32::new(0);

const QOL_TRAY_ID: &str = "qol-tray";

pub(super) fn build_qol_tray_self_with_progress<F>(mut on_progress: F) -> BuildResult
where
    F: FnMut(u8, String),
{
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest_path = repo_root.join("Cargo.toml");
    if let Err(error) = ensure_manifest(&manifest_path) {
        return failed_build(error);
    }
    let CargoChild {
        child,
        stdout,
        stderr,
    } = match start_build(&repo_root, &manifest_path, &mut on_progress) {
        Ok(c) => c,
        Err(result) => return result,
    };
    let readers = artifacts::spawn_readers(stdout, stderr);
    readers.emit_progress(&mut on_progress);
    let (actual_done, combined) = readers.join();
    finish_build(child, actual_done, combined, &mut on_progress)
}

fn start_build<F>(
    repo_root: &Path,
    manifest_path: &Path,
    on_progress: &mut F,
) -> Result<CargoChild, BuildResult>
where
    F: FnMut(u8, String),
{
    log::info!("Building qol-tray from {}", repo_root.display());
    on_progress(2, "Preparing build".to_string());
    spawn_build(repo_root, manifest_path).map_err(failed_build)
}

fn predicted_artifact_count() -> u32 {
    let previous = LAST_ARTIFACT_COUNT.load(Ordering::Relaxed);
    if previous == 0 {
        return 50;
    }
    previous
}

fn ensure_manifest(manifest_path: &Path) -> Result<(), String> {
    if manifest_path.is_file() {
        return Ok(());
    }
    Err(format!(
        "Cargo.toml not found at {}",
        manifest_path.display()
    ))
}

fn spawn_build(repo_root: &Path, manifest_path: &Path) -> Result<CargoChild, String> {
    let mut command = Command::new("cargo");
    command
        .args([
            "build",
            "--bin",
            "qol-tray",
            "--features",
            "dev",
            "--message-format",
            "json",
            "--manifest-path",
        ])
        .arg(manifest_path)
        .current_dir(repo_root);
    spawn_piped(&mut command).map_err(|error| format!("Failed to run cargo build: {}", error))
}

fn finish_build<F>(
    mut child: Child,
    actual_done: u32,
    combined: String,
    on_progress: &mut F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    match child.wait() {
        Ok(status) if status.success() => successful_build(actual_done, combined, on_progress),
        Ok(_) => failed_status_build(combined),
        Err(error) => failed_wait_build(error),
    }
}

fn successful_build<F>(actual_done: u32, combined: String, on_progress: &mut F) -> BuildResult
where
    F: FnMut(u8, String),
{
    LAST_ARTIFACT_COUNT.store(actual_done, Ordering::Relaxed);
    on_progress(100, "Build complete".to_string());
    log::info!("qol-tray build succeeded ({} artifacts)", actual_done);
    finished_build(true, combined)
}

fn failed_status_build(combined: String) -> BuildResult {
    log::error!("qol-tray build failed\n{}", combined);
    finished_build(false, combined)
}

fn failed_wait_build(error: std::io::Error) -> BuildResult {
    let message = format!("Failed while waiting for cargo build: {}", error);
    log::error!("{}", message);
    failed_build(message)
}

fn failed_build(output: String) -> BuildResult {
    BuildResult {
        plugin_id: QOL_TRAY_ID.to_string(),
        success: false,
        output,
        skipped: false,
    }
}

fn finished_build(success: bool, output: String) -> BuildResult {
    BuildResult {
        plugin_id: QOL_TRAY_ID.to_string(),
        success,
        output,
        skipped: false,
    }
}
