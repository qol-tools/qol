mod codesign;
mod plugin_build;
mod self_build;

use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use super::types::BuildResult;
use crate::dev::adapters::CargoPluginBuilder;

pub(super) struct CargoChild {
    pub(super) child: Child,
    pub(super) stdout: ChildStdout,
    pub(super) stderr: ChildStderr,
}

pub(super) fn spawn_piped(command: &mut Command) -> Result<CargoChild, std::io::Error> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    Ok(CargoChild {
        child,
        stdout,
        stderr,
    })
}

pub(super) const BUILD_TIMEOUT: Duration = Duration::from_secs(120);

pub(super) fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Result<bool, String> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("Build timed out after {}s", timeout.as_secs()));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(250)),
            Err(e) => return Err(format!("Failed waiting for build: {}", e)),
        }
    }
}

pub(super) fn finish_build<F>(
    plugin_id: &str,
    child: &mut Child,
    output: String,
    on_progress: &mut F,
    on_success: impl FnOnce(String, &mut F) -> BuildResult,
) -> BuildResult
where
    F: FnMut(u8, String),
{
    match wait_with_timeout(child, BUILD_TIMEOUT) {
        Ok(true) => on_success(output, on_progress),
        Ok(false) => failed_status_build(plugin_id, output),
        Err(message) => failed_build(plugin_id, message),
    }
}

pub(super) fn failed_build(plugin_id: &str, output: String) -> BuildResult {
    build_result(plugin_id, false, output)
}

pub(super) fn failed_status_build(plugin_id: &str, output: String) -> BuildResult {
    log::error!("Cargo build failed for {}:\n{}", plugin_id, output);
    build_result(plugin_id, false, output)
}

pub(super) fn finished_build(plugin_id: &str, output: String) -> BuildResult {
    build_result(plugin_id, true, output)
}

fn build_result(plugin_id: &str, success: bool, output: String) -> BuildResult {
    BuildResult {
        plugin_id: plugin_id.to_string(),
        success,
        output,
        skipped: false,
    }
}

pub(crate) struct CargoCommandPluginBuilder;

impl CargoPluginBuilder for CargoCommandPluginBuilder {
    fn build_plugin_with_progress(
        &self,
        plugin_id: &str,
        path: &Path,
        on_progress: &mut dyn FnMut(u8, String),
    ) -> BuildResult {
        plugin_build::build_cargo_plugin_with_progress(plugin_id, path, on_progress)
    }
}

pub fn build_qol_tray_self_with_progress<F>(repo_root: Option<&Path>, on_progress: F) -> BuildResult
where
    F: FnMut(u8, String),
{
    self_build::build_qol_tray_self_with_progress(repo_root, on_progress)
}

pub fn resolve_qol_tray_self_root(repo_root: Option<&Path>) -> std::path::PathBuf {
    self_build::resolve_qol_tray_self_root(repo_root)
}
