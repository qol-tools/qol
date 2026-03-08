mod progress;
mod streams;

use std::path::Path;
use std::process::{Child, Command};

use super::super::types::BuildResult;
use super::codesign::codesign_debug_binaries;
use super::{spawn_piped, CargoChild};

pub(super) fn build_cargo_plugin_with_progress<F>(
    plugin_id: &str,
    path: &Path,
    mut on_progress: F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    let CargoChild {
        child,
        stdout,
        stderr,
    } = match start_build(plugin_id, path, &mut on_progress) {
        Ok(c) => c,
        Err(result) => return result,
    };
    let readers = streams::spawn_output_readers(stdout, stderr);
    progress::emit_progress(readers.progress_rx(), &mut on_progress);
    let combined = readers.join_output();
    finish_build(plugin_id, path, child, combined, &mut on_progress)
}

fn start_build<F>(
    plugin_id: &str,
    path: &Path,
    on_progress: &mut F,
) -> Result<CargoChild, BuildResult>
where
    F: FnMut(u8, String),
{
    log::info!("Building linked plugin via cargo: {}", plugin_id);
    on_progress(0, "Preparing build".to_string());
    spawn_build(path).map_err(|error| failed_spawn(plugin_id, error))
}

fn spawn_build(path: &Path) -> Result<CargoChild, std::io::Error> {
    let mut command = Command::new("cargo");
    command
        .arg("build")
        .env("CARGO_TERM_PROGRESS_WHEN", "always")
        .env("CARGO_TERM_PROGRESS_WIDTH", "80")
        .env("CARGO_TERM_COLOR", "never")
        .current_dir(path);
    spawn_piped(&mut command)
}

fn finish_build<F>(
    plugin_id: &str,
    path: &Path,
    mut child: Child,
    combined: String,
    on_progress: &mut F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    match super::wait_with_timeout(&mut child, super::BUILD_TIMEOUT) {
        Ok(true) => success_build(plugin_id, path, combined, on_progress),
        Ok(false) => failed_status_build(plugin_id, combined),
        Err(message) => failed_build(plugin_id, message),
    }
}

fn success_build<F>(
    plugin_id: &str,
    path: &Path,
    combined: String,
    on_progress: &mut F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    codesign_debug_binaries(plugin_id, path);
    on_progress(100, "Build complete".to_string());
    log::info!("Cargo build succeeded for {}", plugin_id);
    finished_build(plugin_id, true, combined)
}

fn failed_status_build(plugin_id: &str, combined: String) -> BuildResult {
    log::error!("Cargo build failed for {}:\n{}", plugin_id, combined);
    finished_build(plugin_id, false, combined)
}

fn failed_spawn(plugin_id: &str, error: std::io::Error) -> BuildResult {
    let message = format!("Failed to run cargo build: {}", error);
    log::error!("Build error for {}: {}", plugin_id, message);
    failed_build(plugin_id, message)
}

fn failed_build(plugin_id: &str, output: String) -> BuildResult {
    BuildResult {
        plugin_id: plugin_id.to_string(),
        success: false,
        output,
        skipped: false,
    }
}

fn finished_build(plugin_id: &str, success: bool, output: String) -> BuildResult {
    BuildResult {
        plugin_id: plugin_id.to_string(),
        success,
        output,
        skipped: false,
    }
}
