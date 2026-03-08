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
        mut child,
        stdout,
        stderr,
    } = match start_build(plugin_id, path, &mut on_progress) {
        Ok(c) => c,
        Err(result) => return result,
    };
    let readers = streams::spawn_output_readers(stdout, stderr);
    progress::emit_progress(readers.progress_rx(), &mut on_progress);
    let combined = readers.join_output();
    finish_build(plugin_id, path, &mut child, combined, &mut on_progress)
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
    child: &mut Child,
    combined: String,
    on_progress: &mut F,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    super::finish_build(
        plugin_id,
        child,
        combined,
        on_progress,
        |output, progress| success_build(plugin_id, path, output, progress),
    )
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
    super::finished_build(plugin_id, combined)
}

fn failed_spawn(plugin_id: &str, error: std::io::Error) -> BuildResult {
    let message = format!("Failed to run cargo build: {}", error);
    log::error!("Build error for {}: {}", plugin_id, message);
    super::failed_build(plugin_id, message)
}
