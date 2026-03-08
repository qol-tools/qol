mod artifacts;

use std::path::Path;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicU32, Ordering};

use super::super::types::BuildResult;
use super::{spawn_piped, CargoChild};
use crate::paths;

static LAST_ARTIFACT_COUNT: AtomicU32 = AtomicU32::new(0);

const QOL_TRAY_ID: &str = "qol-tray";

pub(super) fn build_qol_tray_self_with_progress<F>(
    repo_root: Option<&Path>,
    mut on_progress: F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    let repo_root = repo_root
        .map(Path::to_path_buf)
        .unwrap_or_else(paths::repo_root_from_manifest_dir);
    let manifest_path = repo_root.join("Cargo.toml");
    if let Err(error) = ensure_manifest(&manifest_path) {
        return failed_build(error);
    }
    let CargoChild {
        mut child,
        stdout,
        stderr,
    } = match start_build(&repo_root, &manifest_path, &mut on_progress) {
        Ok(c) => c,
        Err(result) => return result,
    };
    let readers = artifacts::spawn_readers(stdout, stderr);
    readers.emit_progress(&mut on_progress);
    let (actual_done, combined) = readers.join();
    finish_build(&mut child, actual_done, combined, &mut on_progress)
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
    child: &mut Child,
    actual_done: u32,
    combined: String,
    on_progress: &mut F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    super::finish_build(
        QOL_TRAY_ID,
        child,
        combined,
        on_progress,
        |output, progress| successful_build(actual_done, output, progress),
    )
}

fn successful_build<F>(actual_done: u32, combined: String, on_progress: &mut F) -> BuildResult
where
    F: FnMut(u8, String),
{
    LAST_ARTIFACT_COUNT.store(actual_done, Ordering::Relaxed);
    on_progress(100, "Build complete".to_string());
    log::info!("qol-tray build succeeded ({} artifacts)", actual_done);
    finished_build(combined)
}

fn failed_build(output: String) -> BuildResult {
    super::failed_build(QOL_TRAY_ID, output)
}

fn finished_build(output: String) -> BuildResult {
    super::finished_build(QOL_TRAY_ID, output)
}
